import { heroActivity } from './hero/activity.mjs'
import { companionFrame, frameDelay, frameImage } from './hero/frames.mjs'
import { xpGain, bubbleFrame, levelUpFrame, visitPose } from './hero/effects.mjs'
import { validateAnimation, validateCharacter, validateLevelUp, validateVisit } from './hero/validate.mjs'
import { formatTokens, windowRect } from './overlay/layout.mjs'

const { invoke } = window.__TAURI__.core
const { listen, emit } = window.__TAURI__.event
const nativeWindow = window.__TAURI__.window.getCurrentWindow()
const canvas = document.getElementById('hero')
const ctx = canvas.getContext('2d')
const stats = document.getElementById('stats')
const menu = document.getElementById('menu')
const activity = heroActivity()
const images = new Map()
let workArea, animation, character, progression, player, sessions = []
let aura, auraStarted = 0, gained, visitor, visitorStarted = 0
let paused = false, hidden = false, screen = 'main', statsOpen = false, menuOpen = false
let timer, rect, lastPlaced, drag, visible = false
let generation = 0
let lastActivityAt = Date.now()

async function bitmap(png) {
  if (!png) return null
  if (!images.has(png)) {
    const bytes = Uint8Array.from(atob(png), char => char.charCodeAt(0))
    images.set(png, createImageBitmap(new Blob([bytes], { type: 'image/png' })).then(image => {
      images.set(png, image)
      return image
    }).catch(error => { images.delete(png); throw error }))
  }
  return images.get(png)
}

async function decodeHero() {
  if (!animation || !character) return
  await Promise.all([
    ...animation.frames,
    ...Object.values(animation.mirroredFrames ?? {}),
    character.png,
    ...(aura?.frames ?? []),
    ...(visitor?.character.animation?.frames ?? []),
    ...Object.values(visitor?.character.animation?.mirroredFrames ?? {}),
  ].map(bitmap))
}

function normaliseActivity(data) {
  const clean = {
    busy: Boolean(data?.busy),
    sessions: Array.isArray(data?.sessions) ? data.sessions.map(s => ({
      ...s,
      state: ({ busy: 'Busy', done: 'Done', idle: 'Idle' })[String(s.state).toLowerCase()] ?? 'Idle',
    })) : [],
  }
  sessions = clean.sessions
  lastActivityAt = Date.now()
  activity.update(clean, lastActivityAt)
  updateStats()
  wake()
}

function tokenCount(data) {
  for (const value of [data?.todayTokens, data?.today_tokens, data?.tokensToday, data?.tokens_today, data?.today?.tokens, data?.today?.totalTokens]) {
    if (Number.isFinite(value) && value >= 0) return value
  }
  return null
}

function updateStats() {
  if (!character) return
  document.getElementById('level').textContent = `Lv ${character.level ?? 1} ${player?.name ?? character.heroId ?? 'Hero'}`
  const xp = character.xp
  const next = character.nextLevelAt
  document.getElementById('xp-text').textContent = Number.isSafeInteger(xp) ? `${formatTokens(xp)} XP${Number.isSafeInteger(next) ? ` / ${formatTokens(next)}` : ''}` : 'XP unavailable'
  document.getElementById('xp-fill').style.width = Number.isSafeInteger(xp) && Number.isSafeInteger(next) && next > 0
    ? `${Math.min(100, 100 * xp / next)}%` : '0%'
  const tokens = tokenCount(progression)
  const tokensLine = document.getElementById('tokens')
  tokensLine.hidden = tokens === null
  if (tokens !== null) tokensLine.textContent = `today ${formatTokens(tokens)} tokens`
  const active = [...new Set(sessions.filter(s => s.state === 'Busy').map(s => s.provider))]
  document.getElementById('providers').textContent = active.length ? active.join(' · ') : 'No active providers'
}

async function setHero(data, heartbeat = false) {
  const now = Date.now()
  const candidate = validateCharacter(data?.character)
  const nextAnimation = validateAnimation(data?.animation ?? candidate?.animation)
  const previous = character
  character = candidate
  animation = nextAnimation
  progression = data?.progression ?? null
  player = data?.player ?? null
  aura = validateLevelUp(data?.levelUp ?? data?.level_up)
  if (heartbeat) {
    gained = xpGain(previous, character, now)
    if (aura && previous?.level < character?.level) auraStarted = now
  }
  visitor = validateVisit(data?.visit, now)
  if (visitor) visitorStarted = now
  const current = ++generation
  if (character && animation) {
    try { await decodeHero() } catch (error) { console.error('hero image decoding:', error); return }
    if (current !== generation) return
    if (!workArea) {
      try { ({ work_area: workArea } = await invoke('hero_geometry', { screen })) }
      catch (error) { console.error('hero geometry:', error); return }
    }
    updateStats()
  } else if (visible) {
    await nativeWindow.hide()
    visible = false
  }
  wake()
}

function drawImage(png, x, y, size, flip = false, opacity = 1) {
  const image = images.get(png)
  if (!image || typeof image.then === 'function') return
  ctx.save()
  ctx.globalAlpha = opacity
  if (flip) {
    ctx.translate(x + size, y)
    ctx.scale(-1, 1)
    ctx.drawImage(image, 0, 0, size, size)
  } else ctx.drawImage(image, x, y, size, size)
  ctx.restore()
}

function deadline(now, pose) {
  const delays = []
  const clip = frameDelay(animation, pose.state, pose.elapsed)
  if (Number.isFinite(clip)) delays.push(clip)
  if (pose.state === 'walk' && !paused) delays.push(83)
  if (pose.state === 'talk') delays.push(Math.max(0, 1500 - pose.elapsed))
  if (pose.state === 'idle') delays.push(Math.max(0, lastActivityAt + 20000 - now))
  if (pose.state === 'sit') delays.push(Math.max(0, lastActivityAt + 90000 - now))
  if (gained && bubbleFrame(gained, now).visible) delays.push(83, Math.max(0, gained.startedAt + 1800 - now))
  if (aura && now - auraStarted < aura.durationMs) delays.push(83, aura.durationMs - (now - auraStarted))
  if (visitor && now - visitorStarted < 8000) delays.push(83, 8000 - (now - visitorStarted))
  return delays.length ? Math.max(83, Math.min(...delays)) : Infinity
}

async function draw() {
  timer = undefined
  if (!character || !animation || !workArea || hidden || paused) return
  const now = Date.now()
  const pose = activity.snapshot(now, { width: Math.max(0, workArea.width - 128) })
  const expanded = statsOpen || menuOpen || Boolean(visitor && now - visitorStarted < 8000)
  const nextRect = windowRect(pose.x, { workArea, bubble: expanded })
  const changed = !lastPlaced || Math.abs(nextRect.x - lastPlaced.x) >= 1 || nextRect.y !== lastPlaced.y || nextRect.width !== lastPlaced.width || nextRect.height !== lastPlaced.height
  rect = nextRect
  if (changed) {
    lastPlaced = nextRect
    try { await invoke('hero_place', { x: rect.x, y: rect.y, width: rect.width, height: rect.height }) }
    catch (error) { console.error('hero placement:', error); return }
  }
  if (!visible) { await nativeWindow.show(); visible = true }
  const ratio = window.devicePixelRatio || 1
  const pixelWidth = Math.round(rect.width * ratio)
  const pixelHeight = Math.round(rect.height * ratio)
  if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
    canvas.width = pixelWidth
    canvas.height = pixelHeight
  }
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0)
  ctx.clearRect(0, 0, rect.width, rect.height)
  ctx.imageSmoothingEnabled = false
  const chosen = frameImage(animation, companionFrame(animation, pose.state, pose.elapsed), pose.facing)
  drawImage(chosen.png ?? character.png, rect.spriteX, rect.spriteY, 128, chosen.flip)
  if (visitor && now - visitorStarted < 8000) {
    const visitAnimation = visitor.character.animation
    const v = visitPose(visitAnimation, now - visitorStarted, 0, Math.max(0, workArea.width - 128))
    const frame = frameImage(visitAnimation, v.frame, v.facing)
    drawImage(frame.png, rect.spriteX + v.offset, rect.spriteY, 128, frame.flip)
  }
  if (aura && now - auraStarted < aura.durationMs) {
    drawImage(aura.frames[levelUpFrame(aura, now - auraStarted)], rect.spriteX, rect.spriteY, 128)
  }
  if (gained) {
    const bubble = bubbleFrame(gained, now)
    if (bubble.visible) {
      ctx.save()
      ctx.globalAlpha = bubble.opacity
      ctx.fillStyle = '#fff3a3'
      ctx.strokeStyle = '#352641'
      ctx.lineWidth = 3
      ctx.font = 'bold 14px monospace'
      const label = `+${formatTokens(gained.amount)} XP`
      const x = rect.spriteX + 64 - ctx.measureText(label).width / 2
      const y = Math.max(16, rect.spriteY + 10 + bubble.dy)
      ctx.strokeText(label, x, y)
      ctx.fillText(label, x, y)
      ctx.restore()
    }
  }
  stats.hidden = !statsOpen || menuOpen
  menu.hidden = !menuOpen
  const next = deadline(now, pose)
  if (Number.isFinite(next)) timer = setTimeout(draw, next)
}

function wake() {
  if (timer) clearTimeout(timer)
  timer = undefined
  void draw()
}

canvas.addEventListener('pointerdown', event => {
  if (!rect || !character || event.button !== 0) return
  const x = event.clientX, y = event.clientY
  if (x < rect.spriteX || x > rect.spriteX + 128 || y < rect.spriteY || y > rect.spriteY + 128) return
  drag = { startX: event.screenX, startY: event.screenY, starting: activity.snapshot(Date.now(), { width: workArea.width - 128 }).x, moved: false }
  canvas.setPointerCapture(event.pointerId)
})
canvas.addEventListener('pointermove', event => {
  if (!drag) return
  if (Math.hypot(event.screenX - drag.startX, event.screenY - drag.startY) > 4) drag.moved = true
  if (drag.moved) {
    activity.drag(drag.starting + event.screenX - drag.startX, Date.now())
    wake()
  }
})
canvas.addEventListener('pointerup', () => {
  if (!drag) return
  if (!drag.moved) { statsOpen = !statsOpen; menuOpen = false; updateStats() }
  drag = null
  wake()
})
canvas.addEventListener('pointercancel', () => { drag = null })
canvas.addEventListener('contextmenu', event => {
  event.preventDefault()
  if (!rect || event.clientX < rect.spriteX || event.clientX > rect.spriteX + 128 || event.clientY < rect.spriteY || event.clientY > rect.spriteY + 128) return
  menuOpen = !menuOpen
  statsOpen = false
  wake()
})
document.getElementById('pause').addEventListener('click', async () => {
  setPaused(true)
  menuOpen = false
  if (timer) clearTimeout(timer)
  timer = undefined
  await emit('overlay-control', { action: 'pause' })
  await nativeWindow.hide()
  visible = false
})
document.getElementById('quit').addEventListener('click', async () => {
  hidden = true
  menuOpen = false
  if (timer) clearTimeout(timer)
  timer = undefined
  await emit('overlay-control', { action: 'quit' })
  await nativeWindow.hide()
  visible = false
})
function setPaused(next) {
  if (next !== paused && workArea) activity.snapshot(Date.now(), { width: Math.max(0, workArea.width - 128), paused: true })
  paused = next
}
window.addEventListener('resize', wake)
setInterval(async () => {
  if (screen !== 'cursor' || paused) return
  try {
    const { work_area: next } = await invoke('hero_geometry', { screen })
    if (!workArea || Object.keys(next).some(key => next[key] !== workArea[key])) {
      workArea = next
      lastPlaced = undefined
      wake()
    }
  } catch (error) { console.error('hero geometry:', error) }
}, 2000)

await listen('daemon', ({ payload }) => {
  if (payload?.kind === 'snapshot') {
    normaliseActivity(payload.data?.activity)
    void setHero(payload.data?.hero)
  } else if (payload?.kind === 'event') {
    if (payload.data?.event === 'activity') normaliseActivity(payload.data.data)
    if (payload.data?.event === 'heartbeat') void setHero(payload.data.data, true)
  } else if (payload?.kind === 'status' && payload.data?.connected === false) {
    normaliseActivity({ busy: false, sessions: [] })
  }
})
const initial = await invoke('desktop_state')
screen = initial.hero?.screen ?? 'main'
setPaused(initial.hero?.show === false)
const snapshot = initial.daemon?.snapshot
if (snapshot) normaliseActivity(snapshot.activity)
if (snapshot) void setHero(snapshot.hero)
await listen('hero-settings', ({ payload }) => {
  const nextScreen = payload?.screen ?? 'main'
  if (screen !== nextScreen) { screen = nextScreen; workArea = undefined; lastPlaced = undefined; void invoke('hero_geometry', { screen }).then(value => { workArea = value.work_area; wake() }) }
  setPaused(payload?.show === false)
  if (paused) { if (timer) clearTimeout(timer); timer = undefined; void nativeWindow.hide(); visible = false }
  else { hidden = false; wake() }
})
await listen('hero-control', ({ payload }) => {
  setPaused(Boolean(payload?.paused))
  if (payload?.show) hidden = false
  if (paused) {
    if (timer) clearTimeout(timer)
    timer = undefined
    void nativeWindow.hide()
    visible = false
  } else wake()
})
