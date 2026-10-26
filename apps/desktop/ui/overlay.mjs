import { heroActivity } from './hero/activity.mjs'
import { companionFrame, frameDelay, frameImage } from './hero/frames.mjs'
import { xpGain, bubbleFrame, levelUpFrame, visitPose, visitPlace } from './hero/effects.mjs'
import { xpTextFrame, drawXpText } from './hero/xp-text.mjs'
import { fallPose } from './hero/fall.mjs'
import { validateAnimation, validateCharacter, validateLevelUp, validateVisit, VISIT_EMOTES, leagueOrBronze } from './hero/validate.mjs'
import { flexPhase, flexDelay, flexPlace, flexMotion, FLEX_DURATION, FLEX_HEIGHT, FLEX_WIDTH } from './hero/flex.mjs'
import { kissHeartPlace } from './hero/kiss.mjs'
import { visitTracker, visitLabelLeft } from './hero/visit.mjs'
import { outgoingVisits, parseOutgoingVisit } from './hero/outgoing.mjs'
import { formatTokens, windowRect } from './overlay/layout.mjs'
import { friendRows, errorText } from './overlay/friends.mjs'

const { invoke } = window.__TAURI__.core
const { listen, emit } = window.__TAURI__.event
const nativeWindow = window.__TAURI__.window.getCurrentWindow()
const canvas = document.getElementById('hero')
const ctx = canvas.getContext('2d')
const stats = document.getElementById('stats')
const menu = document.getElementById('menu')
const emotes = document.getElementById('emotes')
const card = document.getElementById('card')
const friendsPanel = document.getElementById('friends')
const friendsList = document.getElementById('friends-list')
const friendsNote = document.getElementById('friends-note')
const friendsOpenApp = document.getElementById('friends-open')
const visitLabel = document.getElementById('visit-label')
const stillButton = document.getElementById('still')
// The hide shortcut is Control+Option+H on a Mac and Ctrl+Alt+H on Windows (main.rs).
if (/Windows/.test(navigator.userAgent)) document.querySelector('#pause .key').textContent = 'Ctrl+Alt+H'
const EMOTE_LABELS = { flex: 'Flex', kiss: 'Kiss', angry: 'Angry' }
const activity = heroActivity()
const images = new Map()
const effectImages = new Map()
let workArea, animation, character, progression, player, sessions = []
let aura, auraStarted = 0, gained, visitor, visitorStarted = 0
const visits = visitTracker()
const outgoing = outgoingVisits()
const OUTGOING_STORAGE = 'clashofmeat.outgoingVisits.v1'
try {
  const saved = JSON.parse(localStorage.getItem(OUTGOING_STORAGE) ?? '[]')
  if (Array.isArray(saved)) for (const visit of saved) outgoing.observe(visit)
} catch { /* The server heartbeat also recovers an away hero when storage is unavailable. */ }
function saveOutgoing() {
  try {
    const records = outgoing.snapshot()
    if (records.length) localStorage.setItem(OUTGOING_STORAGE, JSON.stringify(records))
    else localStorage.removeItem(OUTGOING_STORAGE)
  } catch { /* The daemon's cached heartbeat is the other restart path. */ }
}
saveOutgoing()
let friendsRequest = 0, sending = false, noteTimer
let paused = false, hidden = false, screen = 'main', statsOpen = false, menuOpen = false, stayStill = false
let timer, rect, lastPlaced, drag, visible = false
let generation = 0
let evolving = false, evolveRetryAt = 0, levelingUp = false, hitKey = ''
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

function effectImage(src) {
  if (!effectImages.has(src)) {
    const image = new Image()
    effectImages.set(src, image)
    image.onload = wake
    image.onerror = () => effectImages.delete(src)
    image.src = src
  }
  const image = effectImages.get(src)
  return image.complete && image.naturalWidth ? image : null
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

// Evolutions happen by themselves, but only while the hero is on screen so the effect is seen.
// The daemon broadcasts the new skin with its effect as a heartbeat; a failure waits a minute.
async function evolve() {
  if (evolving || character?.canLevelUp !== true || Date.now() < evolveRetryAt) return
  evolving = true
  try { await invoke('daemon_request', { method: 'evolve', params: { characterId: character.id, level: character.level } }) }
  catch (error) { console.error('evolve:', error); evolveRetryAt = Date.now() + 60_000 }
  evolving = false
  wake()
}

// Ordinary levels arrive with XP; they get the same gold effect, on the current skin.
async function playLevelUp() {
  if (levelingUp || paused || hidden || !Number.isInteger(character?.evolution)) return
  levelingUp = true
  try {
    const result = await invoke('daemon_request', { method: 'level_up', params: { fromLevel: character.evolution, toLevel: character.evolution } })
    const effect = validateLevelUp(result?.levelUp)
    if (effect) {
      await Promise.all(effect.frames.map(bitmap))
      aura = effect
      auraStarted = Date.now()
      wake()
    }
  } catch (error) { console.error('level up:', error) }
  levelingUp = false
}

function emoteIconSrc(name) {
  return name === 'flex' ? `./assets/leagues/${leagueOrBronze(player?.league)}.png` : `./assets/emote-${name}.png`
}

function emoteButton(name) {
  const button = document.createElement('button')
  const icon = document.createElement('img')
  icon.src = emoteIconSrc(name)
  icon.alt = ''
  // Without the icon file the button falls back to its word.
  icon.addEventListener('error', () => { button.textContent = EMOTE_LABELS[name]; button.style.width = 'auto' }, { once: true })
  button.append(icon)
  button.title = EMOTE_LABELS[name]
  button.setAttribute('aria-label', EMOTE_LABELS[name])
  return button
}

function updateEmotes() {
  emotes.replaceChildren()
  for (const name of VISIT_EMOTES) {
    if (!animation?.clips[name]) continue
    const button = emoteButton(name)
    button.addEventListener('click', () => {
      statsOpen = false
      activity.emote(name, Date.now(), animation.clips[name].durations.reduce((sum, ms) => sum + ms, 0))
      wake()
    })
    emotes.append(button)
  }
}

// The friend panel replaces the card in the same bubble; opening it fetches the list each time.
function showFriends(open) {
  card.hidden = open
  friendsPanel.hidden = !open
  if (open) void loadFriends()
  else friendsRequest++
}

function note(text, { error = false, emote } = {}) {
  clearTimeout(noteTimer)
  friendsNote.replaceChildren()
  friendsNote.hidden = !text
  friendsNote.classList.toggle('error', error)
  if (!text) return
  if (emote) {
    const icon = document.createElement('img')
    icon.src = emoteIconSrc(emote)
    icon.alt = EMOTE_LABELS[emote]
    friendsNote.append(icon)
  }
  friendsNote.append(text)
}

async function loadFriends() {
  const current = ++friendsRequest
  friendsList.replaceChildren()
  friendsOpenApp.hidden = true
  note('Loading friends…')
  let rows
  try { rows = friendRows(await invoke('daemon_request', { method: 'friends', params: {} })) }
  catch (error) {
    if (current === friendsRequest) note(errorText(error), { error: true })
    wake()
    return
  }
  if (current !== friendsRequest) return
  note(rows.length ? null : 'No friends yet — add some on Clash of Meat')
  friendsOpenApp.hidden = rows.length > 0
  for (const friend of rows) friendsList.append(friendRow(friend))
  wake()
}

function friendRow(friend) {
  const row = document.createElement('div')
  row.className = friend.online ? 'friend' : 'friend offline'
  const who = document.createElement('span')
  who.className = 'who'
  who.textContent = friend.online ? friend.name : `${friend.name} (offline)`
  who.title = `@${friend.handle}`
  row.append(who)
  if (friend.online) {
    for (const name of VISIT_EMOTES) {
      const button = emoteButton(name)
      button.title = `${EMOTE_LABELS[name]} to ${friend.name}`
      button.addEventListener('click', () => void sendVisit(friend, name))
      row.append(button)
    }
  }
  return row
}

async function sendVisit(friend, emote) {
  if (sending) return
  sending = true
  const buttons = friendsList.querySelectorAll('button')
  for (const button of buttons) button.disabled = true
  try {
    const result = await invoke('daemon_request', { method: 'send_visit', params: { target: friend.handle, emote } })
    if (!parseOutgoingVisit(result)) throw new Error('The visit was sent without a valid visit ID.')
    outgoing.observe(result)
    saveOutgoing()
    note(null)
    statsOpen = false
    menuOpen = false
    showFriends(false)
  } catch (error) { note(errorText(error), { error: true }) }
  for (const button of buttons) button.disabled = false
  sending = false
  wake()
}

// The daemon acknowledges it with the next heartbeat, and the server stops re-sending it.
function visitDone(id) {
  void invoke('daemon_request', { method: 'visit_done', params: { id } }).catch(error => console.error('visit done:', error))
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
  if (data?.outgoingVisit) {
    outgoing.observe(data.outgoingVisit, now)
    saveOutgoing()
  }
  const candidate = validateCharacter(data?.character)
  const nextAnimation = validateAnimation(data?.animation ?? candidate?.animation)
  const previous = character
  const previousAnimation = animation
  const previousLeague = leagueOrBronze(player?.league)
  character = candidate
  animation = nextAnimation
  progression = data?.progression ?? null
  player = data?.player ?? null
  if (animation !== previousAnimation || leagueOrBronze(player?.league) !== previousLeague) updateEmotes()
  effectImage(`./assets/flex-fx/${leagueOrBronze(player?.league)}.png`)
  effectImage('./assets/emote-kiss.png')
  effectImage('./assets/angry-mark.png')
  // A later heartbeat carries no effect; let a playing aura finish.
  const nextAura = validateLevelUp(data?.levelUp ?? data?.level_up)
  if (nextAura || !(aura && now - auraStarted < aura.durationMs)) aura = nextAura
  if (heartbeat) {
    gained = xpGain(previous, character, now)
    if (aura && previous?.level < character?.level) auraStarted = now
    const recent = aura && now - auraStarted < aura.durationMs + 5000
    if (!nextAura && !recent && previous && character && previous.id === character.id && character.level > previous.level) void playLevelUp()
  }
  // Each visit plays once; a repeat of one that already finished is acknowledged again, in case
  // the last acknowledgement was lost. A hidden hero leaves it for a later heartbeat.
  const visit = validateVisit(data?.visit, now)
  if (visit && !paused && !hidden) {
    if (visits.first(visit.id)) {
      visitor = visit
      visitorStarted = now
      visitLabel.textContent = `from ${visit.name}`
    } else if (visitor?.id !== visit.id) visitDone(visit.id)
  }
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

function drawImage(png, x, y, size, flip = false, opacity = 1, height = size) {
  const image = images.get(png)
  if (!image || typeof image.then === 'function') return
  ctx.save()
  ctx.globalAlpha = opacity
  if (flip) {
    ctx.translate(x + size, y)
    ctx.scale(-1, 1)
    ctx.drawImage(image, 0, 0, size, height)
  } else ctx.drawImage(image, x, y, size, height)
  ctx.restore()
}

function drawEmoteEffect(state, elapsed, frame, spriteX, spriteY, facing, league) {
  if (state === 'flex') {
    const phase = flexPhase(elapsed)
    if (phase < 0) return
    const sheet = effectImage(`./assets/flex-fx/${leagueOrBronze(league)}.png`)
    if (!sheet) return
    const place = flexPlace(spriteX, spriteY, rect.width)
    ctx.drawImage(sheet, 0, phase * FLEX_HEIGHT, FLEX_WIDTH, FLEX_HEIGHT, place.x, place.y, FLEX_WIDTH, FLEX_HEIGHT)
  } else if (state === 'kiss' && frame === 8) {
    const heart = effectImage('./assets/emote-kiss.png')
    if (!heart) return
    const place = kissHeartPlace(spriteX, spriteY, rect.width, facing, elapsed)
    if (!place) return
    ctx.drawImage(heart, place.x, place.y, place.size, place.size)
  } else if (state === 'angry' && frame === 12) {
    const mark = effectImage('./assets/angry-mark.png')
    if (!mark) return
    const pulse = 1 + 0.08 * Math.sin(elapsed / 85)
    const size = 76 * pulse
    const centerX = spriteX + (facing === 'left' ? 35 : 93)
    const x = Math.max(0, Math.min(rect.width - size, centerX - size / 2))
    ctx.drawImage(mark, x, spriteY - 25 - size / 2, size, size)
  }
}

function deadline(now, pose) {
  if (fall) return 16
  const delays = []
  const clip = frameDelay(animation, pose.state, pose.elapsed)
  if (Number.isFinite(clip)) delays.push(clip)
  if (Number.isFinite(pose.until)) delays.push(pose.until)
  if (pose.state === 'walk' && !paused) delays.push(83)
  if (pose.state === 'flex') delays.push(flexDelay(pose.elapsed))
  if (pose.state === 'kiss' || pose.state === 'angry') delays.push(83)
  if (gained && bubbleFrame(gained, now).visible) delays.push(83, Math.max(0, gained.startedAt + 1800 - now))
  if (aura && now - auraStarted < aura.durationMs) delays.push(83, aura.durationMs - (now - auraStarted))
  if (visitor && now - visitorStarted < 8000) {
    delays.push(83, 8000 - (now - visitorStarted))
    const visitorFlexElapsed = now - visitorStarted - 1500
    if (visitor.emote === 'flex' && visitorFlexElapsed >= 0 && visitorFlexElapsed < FLEX_DURATION) delays.push(flexDelay(visitorFlexElapsed))
  }
  return delays.length ? Math.max(16, Math.min(...delays)) : Infinity
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
  if (visitor && now - visitorStarted >= 8000) {
    visitDone(visitor.id)
    visitor = null
  }
  let away = outgoing.away(now)
  const pose = activity.snapshot(now, { width: Math.max(0, workArea.width - 128), still: stayStill || statsOpen || menuOpen || away })
  // One fixed size: the window only moves, so opening the card never resizes it (no flash).
  const nextRect = windowRect(pose.x, { workArea, bubble: true, lift })
  const changed = !lastPlaced || Math.abs(nextRect.x - lastPlaced.x) >= 1 || nextRect.y !== lastPlaced.y || nextRect.width !== lastPlaced.width || nextRect.height !== lastPlaced.height
  rect = nextRect
  if (changed) {
    lastPlaced = nextRect
    try { await invoke('hero_place', { x: rect.x, y: rect.y, width: rect.width, height: rect.height }) }
    catch (error) { console.error('hero placement:', error); return }
  }
  away = outgoing.away(Date.now())
  if (away && !visitor) {
    stats.hidden = true
    menu.hidden = true
    visitLabel.hidden = true
    updateHitRegions()
    if (visible) { await nativeWindow.hide(); visible = false }
    if (!outgoing.away() || visitor) wake()
    return
  }
  if (!visible) { await nativeWindow.show(); visible = true }
  away = outgoing.away(Date.now())
  if (away && !visitor) {
    await nativeWindow.hide()
    visible = false
    if (!outgoing.away() || visitor) wake()
    return
  }
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
  if (!away && auraPlaying) {
    // The effect frames contain the hero turning into its next skin, drawn larger than the sprite
    // (renderScale) with the feet at the bottom, so they replace the sprite while they play.
    const size = 128 * (aura.renderScale ?? 1)
    drawImage(aura.frames[levelUpFrame(aura, now - auraStarted)], rect.spriteX + 64 - size / 2, rect.spriteY + 128 - size, size, pose.facing === 'left')
  } else if (!away) {
    const frame = companionFrame(animation, pose.state, pose.elapsed)
    const chosen = frameImage(animation, frame, pose.facing)
    const motion = pose.state === 'flex' ? flexMotion(pose.elapsed) : { dx: 0, dy: 0, width: 128, height: 128 }
    drawImage(chosen.png ?? character.png, rect.spriteX + motion.dx, rect.spriteY + motion.dy, motion.width, chosen.flip, 1, motion.height)
    drawEmoteEffect(pose.state, pose.elapsed, frame, rect.spriteX, rect.spriteY, pose.facing, player?.league)
  }
  visitLabel.hidden = !visitor
  if (visitor) {
    const visitAnimation = visitor.character.animation
    const place = visitPlace(rect.spriteX, rect.width)
    const v = visitPose(visitAnimation, now - visitorStarted, place.stand, place.edge, visitor.emote)
    const frame = frameImage(visitAnimation, v.frame, v.facing)
    const visitElapsed = now - visitorStarted - 1500
    const motion = v.state === 'flex' ? flexMotion(visitElapsed) : { dx: 0, dy: 0, width: 128, height: 128 }
    drawImage(frame.png, rect.spriteX + v.offset + motion.dx, rect.spriteY + motion.dy, motion.width, frame.flip, 1, motion.height)
    // The visitor's sprite keeps its baked side heart. A second large heart would cover the host.
    if (v.state !== 'kiss') drawEmoteEffect(v.state, visitElapsed, v.frame, rect.spriteX + v.offset, rect.spriteY, v.facing, visitor.league)
    const left = visitLabelLeft(rect.spriteX + v.offset, visitLabel.offsetWidth, rect.width)
    visitLabel.hidden = left === null
    visitLabel.style.left = `${left ?? 0}px`
    visitLabel.style.top = `${rect.spriteY - 4}px`
  }
  if (!away && gained) {
    // Rises 32 px above the head, as in the terminal.
    ctx.save()
    ctx.translate(0, rect.spriteY - 30)
    drawXpText(ctx, xpTextFrame(gained.amount, now - gained.startedAt, { centerX: rect.spriteX + 64, areaHeight: 46, scale: 2, maxWidth: rect.width }))
    ctx.restore()
  }
  if (!away && character.canLevelUp === true && !evolving && !auraPlaying) void evolve()
  stats.hidden = away || !statsOpen || menuOpen
  menu.hidden = away || !menuOpen
  updateHitRegions()
  const next = deadline(now, pose)
  if (Number.isFinite(next)) timer = setTimeout(draw, next)
}

// Only these rectangles take clicks; the rest of the window lets them through (overlay.rs).
function updateHitRegions() {
  const regions = outgoing.away() ? [] : drag ? [[0, 0, rect.width, rect.height]] : [[rect.spriteX + 8, rect.spriteY + 8, 112, 120]]
  for (const element of [stats, menu]) {
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
  if (!rect || !character || outgoing.away() || event.button !== 0) return
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
    // Held in the air, the authored grab poses flail until the hero is let go.
    play('grab', Date.now(), 600_000)
  }
  if (drag.moved) {
    activity.drag(drag.starting + event.screenX - drag.startX, Date.now())
    lift = Math.max(0, Math.min(workArea.height - 128, drag.startLift + drag.startY - event.screenY))
    wake()
  }
})
function release() {
  if (!drag) return
  if (!drag.moved) {
    statsOpen = !statsOpen
    menuOpen = false
    // The bubble always opens on the hero card.
    if (statsOpen) showFriends(false)
    updateStats()
  }
  else {
    play('grab', Date.now(), 0)
    if (lift >= 2) fall = { from: lift, startedAt: Date.now() }
  }
  drag = null
  wake()
}
canvas.addEventListener('pointerup', release)
canvas.addEventListener('pointercancel', release)
canvas.addEventListener('contextmenu', event => {
  event.preventDefault()
  if (outgoing.away()) return
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
stillButton.addEventListener('click', async () => {
  menuOpen = false
  const next = !stayStill
  setStayStill(next)
  try { await invoke('set_hero_settings', { show: true, screen, still: next }) }
  catch (error) { console.error('hero settings:', error); setStayStill(!next) }
})
document.getElementById('open-app').addEventListener('click', async () => {
  statsOpen = false
  wake()
  try { await invoke('open_app', { path: '/heroes' }) }
  catch (error) { console.error('open Clash of Meat:', error) }
})
document.getElementById('send-friend').addEventListener('click', () => { showFriends(true); wake() })
document.getElementById('friends-back').addEventListener('click', () => { showFriends(false); wake() })
friendsOpenApp.addEventListener('click', async () => {
  statsOpen = false
  wake()
  try { await invoke('open_app', { path: '/friends' }) }
  catch (error) { console.error('open Clash of Meat:', error) }
})
document.getElementById('settings').addEventListener('click', async () => {
  menuOpen = false
  wake()
  await emit('overlay-control', { action: 'settings' })
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
let checkingOutgoing = false
setInterval(async () => {
  const now = Date.now()
  if (outgoing.expire(now)) { saveOutgoing(); wake() }
  if (checkingOutgoing) return
  const ids = outgoing.ids(now)
  if (!ids.length) return
  checkingOutgoing = true
  try {
    for (const id of ids) {
      try {
        const result = await invoke('daemon_request', { method: 'visit_status', params: { id } })
        const changed = outgoing.observe(result?.visit)
        if (result?.visit) saveOutgoing()
        if (changed) wake()
      } catch { /* An offline sender stays away until the visit expires locally. */ }
    }
  } finally { checkingOutgoing = false }
}, 3000)
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
const firstHero = initial.daemon?.heartbeat ?? snapshot?.hero
if (firstHero) void setHero(firstHero)
// A cached snapshot may predate the visit. Refresh it instead of waiting for the idle minute.
if (initial.daemon?.connected) void invoke('daemon_request', { method: 'sync_now', params: {} }).catch(() => {})
await listen('hero-settings', ({ payload }) => {
  setStayStill(payload?.still === true)
  const nextScreen = payload?.screen ?? 'main'
  if (screen !== nextScreen) { screen = nextScreen; workArea = undefined; lastPlaced = undefined; void invoke('hero_geometry', { screen }).then(value => { workArea = value.work_area; wake() }) }
  setPaused(payload?.show === false)
  if (paused) { if (timer) clearTimeout(timer); timer = undefined; void nativeWindow.hide(); visible = false }
  else { hidden = false; wake() }
})
// A click elsewhere on screen near the hero (overlay.rs reports it in window coordinates) sends it
// hurrying away, so it never sits on what the user is reaching for. "Stop walking" keeps it put.
await listen('screen-click', ({ payload }) => {
  if (!rect || !character || outgoing.away() || paused || hidden || stayStill || drag || fall || statsOpen || menuOpen) return
  if (aura && Date.now() - auraStarted < aura.durationMs) return
  const dx = payload.x - (rect.spriteX + 64), dy = payload.y - (rect.spriteY + 64)
  if (Math.hypot(dx, dy) > 260) return
  activity.flee(dx, Date.now())
  wake()
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
