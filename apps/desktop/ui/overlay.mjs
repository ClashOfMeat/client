import { heroActivity } from './hero/activity.mjs'
import { companionFrame, frameDelay, frameImage } from './hero/frames.mjs'
import { xpGain, bubbleFrame, levelUpFrame, visitPose } from './hero/effects.mjs'
import { xpTextFrame, levelUpTextFrame, drawXpText } from './hero/xp-text.mjs'
import { fallPose } from './hero/fall.mjs'
import { validateAnimation, validateCharacter, validateLevelUp, validateVisit } from './hero/validate.mjs'
import { formatTokens, windowRect } from './overlay/layout.mjs'

const { invoke } = window.__TAURI__.core
const { listen, emit } = window.__TAURI__.event
const nativeWindow = window.__TAURI__.window.getCurrentWindow()
const canvas = document.getElementById('hero')
const ctx = canvas.getContext('2d')
const stats = document.getElementById('stats')
const menu = document.getElementById('menu')
const emotes = document.getElementById('emotes')
const stillButton = document.getElementById('still')
const evolveButton = document.getElementById('evolve')
const levelupButton = document.getElementById('levelup')
const EMOTE_LABELS = { wave: 'Wave', laugh: 'Laugh', victory: 'Victory', kiss: 'Kiss', angry: 'Angry' }
const activity = heroActivity()
const images = new Map()
let workArea, animation, character, progression, player, sessions = []
let aura, auraStarted = 0, gained, visitor, visitorStarted = 0
let paused = false, hidden = false, screen = 'main', statsOpen = false, menuOpen = false, stayStill = false
let timer, rect, lastPlaced, drag, visible = false
let generation = 0
let evolving = false, hitKey = ''
// Height of the feet above the ground while carried or falling, and the fall in progress.
let lift = 0, fall = null
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
  document.getElementById('name').textContent = character.name ?? heroName(character.heroId)
  const level = `Level ${character.level ?? 1}`
  const xp = character.xp
  const next = character.nextLevelAt
  const floor = Number.isSafeInteger(character.levelFloor) ? character.levelFloor : 0
  const span = next - floor
  const into = xp - floor
  const fmt = value => value.toLocaleString('en-US')
  const xpText = document.getElementById('xp-text')
  const xpFill = document.getElementById('xp-fill')
  if (Number.isSafeInteger(xp) && Number.isSafeInteger(next) && span > 0) {
    xpText.textContent = `${level} · ${fmt(Math.min(into, span))} / ${fmt(span)} XP`
    xpFill.style.width = `${Math.max(0, Math.min(1, into / span)) * 100}%`
  } else if (Number.isSafeInteger(xp) && (next === null || span <= 0)) {
    xpText.textContent = `${level} · ${fmt(xp)} XP · max level`
    xpFill.style.width = '100%'
  } else {
    xpText.textContent = 'XP unavailable'
    xpFill.style.width = '0%'
  }
  evolveButton.hidden = character.canLevelUp !== true
  evolveButton.disabled = evolving
  evolveButton.textContent = evolving ? 'Evolving…' : `★ Evolve to Lv ${(character.level ?? 1) + 1}`
  const tokens = tokenCount(progression)
  const tokensLine = document.getElementById('tokens')
  tokensLine.hidden = tokens === null
  if (tokens !== null) tokensLine.textContent = `today ${formatTokens(tokens)} tokens`
  const active = [...new Set(sessions.filter(s => s.state === 'Busy').map(s => s.provider))]
  document.getElementById('providers').textContent = active.length ? active.join(' · ') : 'No agent working'
}

// Older servers send no display name; "framework_familiar" reads as "Framework Familiar".
function heroName(id) {
  return id ? id.split('_').map(word => word[0].toUpperCase() + word.slice(1)).join(' ') : 'Hero'
}

// The server checks the level and plays the effect; the daemon then broadcasts the new skin as a heartbeat.
async function evolve() {
  if (evolving || character?.canLevelUp !== true) return
  evolving = true
  statsOpen = false
  updateStats()
  wake()
  try { await invoke('daemon_request', { method: 'evolve', params: { characterId: character.id, level: character.level } }) }
  catch (error) { console.error('evolve:', error) }
  evolving = false
  updateStats()
  wake()
}

function updateEmotes() {
  emotes.replaceChildren()
  for (const name of ['wave', 'laugh', 'victory', 'kiss', 'angry']) {
    if (!animation?.clips[name]) continue
    const button = document.createElement('button')
    const icon = document.createElement('img')
    icon.src = `./assets/emote-${name}.png`
    icon.alt = ''
    // Without the icon file the button falls back to its word.
    icon.addEventListener('error', () => { button.textContent = EMOTE_LABELS[name]; button.style.width = 'auto' }, { once: true })
    button.append(icon)
    button.title = EMOTE_LABELS[name]
    button.setAttribute('aria-label', EMOTE_LABELS[name])
    button.addEventListener('click', () => {
      statsOpen = false
      activity.emote(name, Date.now(), animation.clips[name].durations.reduce((sum, ms) => sum + ms, 0))
      wake()
    })
    emotes.append(button)
  }
}

function setStayStill(next) {
  if (next && !stayStill && workArea) {
    const now = Date.now()
    activity.drag(activity.snapshot(now, { width: Math.max(0, workArea.width - 128), still: true }).x, now)
  }
  if (!next && stayStill && workArea) activity.strollNow(Date.now())
  stayStill = next
  stillButton.textContent = stayStill ? 'Start walking' : 'Stop walking'
  wake()
}

async function setHero(data, heartbeat = false) {
  const now = Date.now()
  const candidate = validateCharacter(data?.character)
  const nextAnimation = validateAnimation(data?.animation ?? candidate?.animation)
  const previous = character
  const previousAnimation = animation
  character = candidate
  animation = nextAnimation
  if (animation !== previousAnimation) updateEmotes()
  progression = data?.progression ?? null
  player = data?.player ?? null
  // A later heartbeat carries no effect; let a playing aura finish.
  const nextAura = validateLevelUp(data?.levelUp ?? data?.level_up)
  if (nextAura || !(aura && now - auraStarted < aura.durationMs)) aura = nextAura
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
  if (fall) return 16
  const delays = []
  const clip = frameDelay(animation, pose.state, pose.elapsed)
  if (Number.isFinite(clip)) delays.push(clip)
  if (Number.isFinite(pose.until)) delays.push(pose.until)
  if (pose.state === 'walk' && !paused) delays.push(83)
  if (gained && bubbleFrame(gained, now).visible) delays.push(83, Math.max(0, gained.startedAt + 1800 - now))
  if (aura && now - auraStarted < aura.durationMs) delays.push(83, aura.durationMs - (now - auraStarted))
  if (visitor && now - visitorStarted < 8000) delays.push(83, 8000 - (now - visitorStarted))
  return delays.length ? Math.max(83, Math.min(...delays)) : Infinity
}

async function draw() {
  timer = undefined
  if (!character || !animation || !workArea || hidden || paused) return
  const now = Date.now()
  if (fall) {
    const step = fallPose(fall.from, now - fall.startedAt)
    lift = step.lift
    if (step.landed) {
      fall = null
      lift = 0
      play('sit', now, 3000)
    }
  }
  const pose = activity.snapshot(now, { width: Math.max(0, workArea.width - 128), still: stayStill || statsOpen || menuOpen })
  // One fixed size: the window only moves, so opening the card never resizes it (no flash).
  const nextRect = windowRect(pose.x, { workArea, bubble: true, lift })
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
  const auraPlaying = Boolean(aura && now - auraStarted < aura.durationMs)
  if (auraPlaying) {
    // The effect frames contain the hero turning into its next skin, drawn larger than the sprite
    // (renderScale) with the feet at the bottom, so they replace the sprite while they play.
    const size = 128 * (aura.renderScale ?? 1)
    drawImage(aura.frames[levelUpFrame(aura, now - auraStarted)], rect.spriteX + 64 - size / 2, rect.spriteY + 128 - size, size, pose.facing === 'left')
  } else {
    const chosen = frameImage(animation, companionFrame(animation, pose.state, pose.elapsed), pose.facing)
    drawImage(chosen.png ?? character.png, rect.spriteX, rect.spriteY, 128, chosen.flip)
  }
  if (visitor && now - visitorStarted < 8000) {
    const visitAnimation = visitor.character.animation
    const v = visitPose(visitAnimation, now - visitorStarted, 0, Math.max(0, workArea.width - 128))
    const frame = frameImage(visitAnimation, v.frame, v.facing)
    drawImage(frame.png, rect.spriteX + v.offset, rect.spriteY, 128, frame.flip)
  }
  if (gained) {
    // Rises 32 px above the head, as in the terminal.
    ctx.save()
    ctx.translate(0, rect.spriteY - 30)
    drawXpText(ctx, xpTextFrame(gained.amount, now - gained.startedAt, { centerX: rect.spriteX + 64, areaHeight: 46, scale: 2, maxWidth: rect.width }))
    ctx.restore()
  }
  levelupButton.hidden = character.canLevelUp !== true || evolving || auraPlaying
  if (!levelupButton.hidden) {
    levelupButton.style.left = `${Math.max(0, Math.min(rect.width - 160, rect.spriteX + 64 - 80))}px`
    levelupButton.style.top = `${Math.max(0, rect.spriteY - 40)}px`
  }
  stats.hidden = !statsOpen || menuOpen
  menu.hidden = !menuOpen
  updateHitRegions()
  const next = deadline(now, pose)
  if (Number.isFinite(next)) timer = setTimeout(draw, next)
}

// Only these rectangles take clicks; the rest of the window lets them through (overlay.rs).
function updateHitRegions() {
  const regions = drag ? [[0, 0, rect.width, rect.height]] : [[rect.spriteX + 8, rect.spriteY + 8, 112, 120]]
  for (const element of [levelupButton, stats, menu]) {
    if (element.hidden) continue
    const box = element.getBoundingClientRect()
    regions.push([box.left, box.top, box.width, box.height])
  }
  const key = JSON.stringify(regions)
  if (key === hitKey) return
  hitKey = key
  void invoke('hero_hit', { regions }).catch(error => console.error('hero hit regions:', error))
}

// Plays one of the hero's clips, if this hero has it.
function play(name, at, durationMs) {
  if (animation?.clips[name]) activity.emote(name, at, durationMs)
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
  // Grabbing a falling hero catches it where it is.
  fall = null
  drag = { startX: event.screenX, startY: event.screenY, startLift: lift, starting: activity.snapshot(Date.now(), { width: workArea.width - 128, still: true }).x, moved: false }
  canvas.setPointerCapture(event.pointerId)
  updateHitRegions()
})
canvas.addEventListener('pointermove', event => {
  if (!drag) return
  if (!drag.moved && Math.hypot(event.screenX - drag.startX, event.screenY - drag.startY) > 4) {
    drag.moved = true
    statsOpen = false
    menuOpen = false
    // Held in the air, the hero struggles until it is let go.
    play('angry', Date.now(), 600_000)
  }
  if (drag.moved) {
    activity.drag(drag.starting + event.screenX - drag.startX, Date.now())
    lift = Math.max(0, Math.min(workArea.height - 128, drag.startLift + drag.startY - event.screenY))
    wake()
  }
})
function release() {
  if (!drag) return
  if (!drag.moved) { statsOpen = !statsOpen; menuOpen = false; updateStats() }
  else if (lift >= 2) fall = { from: lift, startedAt: Date.now() }
  else play('angry', Date.now(), 0)
  drag = null
  wake()
}
canvas.addEventListener('pointerup', release)
canvas.addEventListener('pointercancel', release)
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
evolveButton.addEventListener('click', () => { void evolve() })
levelupButton.addEventListener('click', () => { void evolve() })
// The plaque's words are drawn once in the same 5x7 pixel font as the XP popup.
{
  const text = document.getElementById('levelup-text'), ratio = window.devicePixelRatio || 1
  const frame = { ...levelUpTextFrame({ centerX: 53, top: 0 }), left: 0, color: '#3b2203' }
  text.width = frame.width * ratio
  text.height = frame.height * ratio
  text.style.width = `${frame.width}px`
  text.style.height = `${frame.height}px`
  const pen = text.getContext('2d')
  pen.scale(ratio, ratio)
  drawXpText(pen, frame)
}
stillButton.addEventListener('click', async () => {
  menuOpen = false
  const next = !stayStill
  setStayStill(next)
  try { await invoke('set_hero_settings', { show: true, screen, still: next }) }
  catch (error) { console.error('hero settings:', error); setStayStill(!next) }
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
  if (next !== paused && workArea) activity.snapshot(Date.now(), { width: Math.max(0, workArea.width - 128), paused: true, still: true })
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
setStayStill(initial.hero?.still === true)
setPaused(initial.hero?.show === false)
const snapshot = initial.daemon?.snapshot
if (snapshot) normaliseActivity(snapshot.activity)
if (snapshot) void setHero(snapshot.hero)
await listen('hero-settings', ({ payload }) => {
  setStayStill(payload?.still === true)
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
