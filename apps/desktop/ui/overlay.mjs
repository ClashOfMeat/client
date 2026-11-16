import { heroActivity } from './hero/activity.mjs'
import { companionFrame, frameDelay, frameImage } from './hero/frames.mjs'
import { decodePng } from './hero/png-image.mjs'
import { xpGain, bubbleFrame, levelUpFrame, visitPose, visitDuration, visitLabelVisible } from './hero/effects.mjs'
import { xpTextFrame, drawXpText } from './hero/xp-text.mjs'
import { fallPose } from './hero/fall.mjs'
import { validateAnimation, validateCharacter, validateLevelUp, validateVisit, VISIT_EMOTES, leagueOrBronze } from './hero/validate.mjs'
import { flexPhase, flexDelay, flexLabel, flexLabelMaxWidth, flexPlace, visitorFlexPlace, flexMotion, FLEX_DURATION, FLEX_HEIGHT, FLEX_WIDTH } from './hero/flex.mjs'
import { kissHeartPlace, visitorKissHeartPlace } from './hero/kiss.mjs'
import { visitTracker, visitLabelLeft, visitAnchor, anchoredRect, edgeFade } from './hero/visit.mjs'
import { scopedOutgoingVisits, parseOutgoingVisit } from './hero/outgoing.mjs'
import { beginDeparture, departurePose } from './hero/departure.mjs'
import { windowRect } from './overlay/layout.mjs'
import { HERO_CELL_SIZE, LEGACY_FRAME_SIZE, heroDrawRect, heroEffectAnchor } from './hero/display.mjs'
import { heroSettings } from './settings/hero.mjs'
import { SIGN_EMOTE, SIGN_DROP_AT, signPostX } from './hero/plant.mjs'
import { checkReviewLink } from './overlay/review-link.mjs'
import { HERO_CONTRACT } from './hero/hero-contract.mjs'

// The hero window has no text to select or images to drag; a selection there could never be cleared (see overlay.css).
for (const type of ['selectstart', 'dragstart']) document.addEventListener(type, event => { if (event.target !== signUrl) event.preventDefault() })
document.getSelection()?.removeAllRanges()
import { friendRows, friendPage, errorText } from './overlay/friends.mjs'
import { predict, levelDue, steadyXp } from './overlay/predict.mjs'
import { xpBar } from './overlay/xp-bar.mjs'
import { notificationSnapshot, bellCount, notificationIcon, bellPosition, bellVisible, unreadCount, markedRead, stillSelected, arrived, BELL_RING_MS } from './overlay/notifications.mjs'

const { invoke } = window.__TAURI__.core
const { listen, emit } = window.__TAURI__.event
const nativeWindow = window.__TAURI__.window.getCurrentWindow()
const canvas = document.getElementById('hero')
const ctx = canvas.getContext('2d')
const stats = document.getElementById('stats')
const menu = document.getElementById('menu')
const card = document.getElementById('card')
const friendsPanel = document.getElementById('friends')
const friendsList = document.getElementById('friends-list')
const friendsNote = document.getElementById('friends-note')
const friendsOpenApp = document.getElementById('friends-open')
const friendsPager = document.getElementById('friends-pager')
const friendsPrev = document.getElementById('friends-prev')
const friendsNext = document.getElementById('friends-next')
const friendsPageLabel = document.getElementById('friends-page')
const notificationBell = document.getElementById('notification-bell')
const notificationCount = document.getElementById('notification-count')
const notificationsPanel = document.getElementById('notifications')
const notificationsList = document.getElementById('notifications-list')
const notificationsRead = document.getElementById('notifications-read')
const cardBell = document.getElementById('card-bell')
const cardBellCount = document.getElementById('card-bell-count')
const notificationsNote = document.getElementById('notifications-note')
const visitLabel = document.getElementById('visit-label')
const stillButton = document.getElementById('still')
const signStep = document.getElementById('sign-step')
const signUrl = document.getElementById('sign-url')
const signWhy = document.getElementById('sign-why')
const signSend = document.getElementById('sign-send')
const removeSigns = document.getElementById('remove-signs')
const EMOTE_LABELS = { flex: 'Flex', kiss: 'Kiss', angry: 'Angry', [SIGN_EMOTE]: 'Review me' }
// A paid emote this app has no button art for shows its key as words ("happy_dance" reads "Happy dance").
const emoteLabel = key => EMOTE_LABELS[key] ?? key.replace(/_/g, ' ').replace(/^./, c => c.toUpperCase())
const WALK_IN = HERO_CONTRACT.visit.walkInMs
const EMOTE_KEY = /^[a-z][a-z0-9_]{1,30}$/
const activity = heroActivity()
const images = new Map()
const effectImages = new Map()
let workArea, animation, character, progression, player
// This machine's tokens by kind: those the last heartbeat did not include yet, and those since UTC
// midnight (see overlay/predict.mjs). An older daemon sends neither, and the hero then shows the
// server's numbers alone.
let localPending = null
let localToday = null
// What the XP bar last showed (see steadyXp), and the syncs asked for a level that is due.
let shownXp = null, levelAsk = { key: '', tries: 0, at: 0 }
// visitSpot pins the window and the visitor where the visitor arrived (see visitAnchor).
let aura, auraStarted = 0, gained, visitor, visitorStarted = 0, visitSpot = null, departure = null
const visits = visitTracker()
let outgoingStorage
try {
  outgoingStorage = localStorage
  // Old records had no account owner. A heartbeat restores an active visit under the new key.
  outgoingStorage.removeItem('clashofmeat.outgoingVisits.v1')
} catch { outgoingStorage = { getItem: () => null, setItem: () => {}, removeItem: () => {} } }
const outgoing = scopedOutgoingVisits(outgoingStorage)
let friendsRequest = 0, sending = false, noteTimer
let friends = [], friendsPageIndex = 0
// The emotes pinned on the active hero (the friends list brings them), one button each in a friend's row.
let pins = [...VISIT_EMOTES]
// The friend a Review me sign is going to while its link step is open, and whether the hero window has the keyboard.
let signTarget = null, keyboard = false
// Friends' signs planted on this desktop (signs.mjs counts them), and whether they are allowed (Settings).
let signCount = 0, allowSigns = true
let inbox = null, inboxOwner = null, notificationBusy = false, notificationError = '', inboxEpoch = 0
// Unread rows ticked in the inbox, waiting for "Mark as read".
let selectedNotifications = new Set()
// The bell beside the hero rings until this time after a new arrival; lastUnread is what it last saw.
let bellRingUntil = 0, lastUnread = null
let paused = false, hidden = false, screen = 'main', statsOpen = false, menuOpen = false, stayStill = true
let timer, rect, lastPlaced, drag, visible = false
let generation = 0
let evolving = false, evolveRetryAt = 0, levelingUp = false, hitKey = ''
// Height of the feet above the ground while carried or falling, and the fall in progress.
let lift = 0, fall = null
let lastActivityAt = Date.now()

// Decoded <img> elements, not ImageBitmaps: those draw nothing after WebKit's GPU process restarts (hero/png-image.mjs).
async function heroImage(png) {
  if (!png) return null
  if (!images.has(png)) {
    images.set(png, decodePng(png).then(image => {
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

// Every PNG the overlay may draw for this hero: its skin and frames, its effect and a visiting friend.
function heroPngs(hero = character, frames = animation, effect = aura) {
  return [
    ...frames.frames,
    ...Object.values(frames.mirroredFrames ?? {}),
    hero.png,
    ...(effect?.frames ?? []),
    ...(visitor?.character.animation?.frames ?? []),
    ...Object.values(visitor?.character.animation?.mirroredFrames ?? {}),
  ]
}

async function decodeHero() {
  if (!animation || !character) return
  await Promise.all(heroPngs().map(heroImage))
}

function normaliseActivity(data) {
  const clean = {
    busy: Boolean(data?.busy),
    sessions: Array.isArray(data?.sessions) ? data.sessions.map(s => ({
      ...s,
      state: ({ busy: 'Busy', done: 'Done', idle: 'Idle' })[String(s.state).toLowerCase()] ?? 'Idle',
    })) : [],
  }
  lastActivityAt = Date.now()
  activity.update(clean, lastActivityAt)
  updateStats()
  wake()
}

function updateStats() {
  if (!character) return
  document.getElementById('name').textContent = character.name ?? heroName(character.heroId)
  shownXp = steadyXp(shownXp, character, predict(character, progression, localPending, localToday).xp)
  const bar = xpBar(character, shownXp.xp)
  confirmLevel()
  document.getElementById('xp-text').textContent = bar.level
  document.getElementById('xp-count').textContent = bar.count
  document.getElementById('xp-fill').style.width = `${bar.fill}%`
  document.querySelector('.xp-track').dataset.state = bar.state
}

// Local tokens already pay for the next level, but only the server grants it and the bar waits
// one XP short. A sync uploads them and the one after brings the level, so ask a few times, ten
// seconds apart, instead of sitting there until the next heartbeats.
function confirmLevel() {
  const key = `${character.id}:${character.level}`
  if (levelAsk.key !== key) levelAsk = { key, tries: 0, at: 0 }
  if (!levelDue(character, progression, localPending) || levelAsk.tries >= 3 || Date.now() - levelAsk.at < 10_000) return
  levelAsk.tries++
  levelAsk.at = Date.now()
  void invoke('daemon_request', { method: 'sync_now', params: {} }).catch(() => {})
  // The bar stops being refreshed once local tokens stop arriving; look again by ourselves.
  setTimeout(updateStats, 10_500)
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
      await Promise.all(effect.frames.map(heroImage))
      aura = effect
      auraStarted = Date.now()
      wake()
    }
  } catch (error) { console.error('level up:', error) }
  levelingUp = false
}

function emoteIconSrc(name) {
  if (name === SIGN_EMOTE) return './assets/sign-review-me.png'
  return name === 'flex' ? `./assets/leagues/${leagueOrBronze(player?.league)}.png` : `./assets/emote-${name}.png`
}

function emoteButton(name) {
  const button = document.createElement('button')
  const icon = document.createElement('img')
  icon.src = emoteIconSrc(name)
  icon.alt = ''
  if (name === SIGN_EMOTE) icon.className = 'sign-icon'
  // Without the icon file the button falls back to its word.
  icon.addEventListener('error', () => { button.textContent = emoteLabel(name); button.style.width = 'auto' }, { once: true })
  button.append(icon)
  button.title = emoteLabel(name)
  button.setAttribute('aria-label', emoteLabel(name))
  return button
}

// The friend panel replaces the card in the same bubble; opening it fetches the list each time.
function showFriends(open) {
  card.hidden = open
  friendsPanel.hidden = !open
  notificationsPanel.hidden = true
  signStep.hidden = true
  if (open) void loadFriends()
  else friendsRequest++
}

function showNotifications(open) {
  card.hidden = open
  friendsPanel.hidden = true
  signStep.hidden = true
  notificationsPanel.hidden = !open
  friendsRequest++
  if (open) {
    renderNotifications()
    void invoke('daemon_request', { method: 'sync_now', params: {} }).catch(() => {})
  }
}

function receiveNotifications(value) {
  const next = notificationSnapshot(value)
  if (!next) return
  inbox = next
  notificationError = ''
  renderNotifications()
}

function notificationHeartbeat(data) {
  const owner = data?.player?.handle
  if (typeof owner === 'string' && inboxOwner !== null && inboxOwner !== owner) {
    inboxEpoch++
    inbox = null
    notificationError = ''
    renderNotifications()
  }
  if (typeof owner === 'string') inboxOwner = owner
  if (data && Object.hasOwn(data, 'notifications')) receiveNotifications(data.notifications)
}

function renderNotifications() {
  const count = unreadCount(inbox)
  const badge = bellCount(count)
  if (arrived(lastUnread, count)) ringBell()
  lastUnread = count
  for (const [bell, counter] of [[notificationBell, notificationCount], [cardBell, cardBellCount]]) {
    counter.hidden = !badge
    counter.textContent = badge
    bell.setAttribute('aria-label', count ? `Notifications, ${count} unread` : 'Notifications')
  }
  selectedNotifications = stillSelected(inbox, selectedNotifications)
  notificationsRead.disabled = notificationBusy || selectedNotifications.size === 0
  notificationsList.replaceChildren(...(inbox?.items ?? []).map(notificationRow))
  const message = notificationError || (!inbox ? 'Notifications will appear after the next sync.'
    : inbox.items.length ? '' : "You're all caught up.")
  notificationsNote.hidden = !message
  notificationsNote.textContent = message
  notificationsNote.classList.toggle('error', Boolean(notificationError))
  // The bell appears or disappears with the count, so redraw even with the inbox closed.
  wake()
}

function showBell(away = outgoing.away()) {
  notificationBell.hidden = !bellVisible({
    count: unreadCount(inbox), ringing: Date.now() < bellRingUntil, away, carried: Boolean(drag?.moved),
  })
}

// Restarts the ring-and-fade animation; draw() hides the bell once it is over.
function ringBell() {
  bellRingUntil = Date.now() + BELL_RING_MS
  notificationBell.classList.remove('ringing')
  void notificationBell.offsetWidth
  notificationBell.classList.add('ringing')
}

function notificationRow(item) {
  const entry = document.createElement('div')
  entry.className = 'notification-item'
  const row = document.createElement('button')
  row.type = 'button'
  row.className = 'notification-row'
  const icon = document.createElement('img')
  icon.className = 'notification-icon'
  icon.src = notificationIcon(item.kind)
  icon.alt = ''
  const copy = document.createElement('span')
  copy.className = 'notification-copy'
  copy.append(item.sentence)
  const time = document.createElement('time')
  time.dateTime = item.at
  time.textContent = new Date(item.at).toLocaleString('en-US')
  copy.append(time)
  row.append(icon, copy)
  if (item.claimable) {
    const dot = document.createElement('span')
    dot.className = 'notification-dot'
    dot.setAttribute('role', 'img')
    dot.setAttribute('aria-label', 'Reward to claim')
    row.append(dot)
  }
  row.addEventListener('click', () => void openNotification(item))
  entry.append(row)
  // An unread row can be ticked (the box shows on hover) and read with the others from the header.
  if (!item.read) {
    const pick = document.createElement('input')
    pick.type = 'checkbox'
    pick.className = 'notification-pick'
    pick.checked = selectedNotifications.has(item.id)
    pick.title = 'Select to mark as read'
    pick.setAttribute('aria-label', `Select: ${item.sentence}`)
    pick.addEventListener('change', () => {
      if (pick.checked) selectedNotifications.add(item.id)
      else selectedNotifications.delete(item.id)
      notificationsRead.disabled = notificationBusy || selectedNotifications.size === 0
    })
    entry.append(pick)
  }
  return entry
}

// The server reads one row per request; the ticked rows leave the list at once and come back if it refuses.
async function readSelectedNotifications() {
  if (notificationBusy || !inbox || !selectedNotifications.size) return
  notificationBusy = true
  const epoch = inboxEpoch
  const before = inbox
  const ids = [...selectedNotifications]
  inbox = markedRead(inbox, ids)
  selectedNotifications = new Set()
  notificationError = ''
  renderNotifications()
  try {
    let next
    for (const id of ids) {
      next = await invoke('daemon_request', { method: 'notifications_read', params: { id } })
      if (epoch !== inboxEpoch) break
    }
    if (epoch === inboxEpoch) receiveNotifications(next)
  } catch (error) {
    if (epoch === inboxEpoch) { inbox = before; notificationError = errorText(error) }
  }
  notificationBusy = false
  if (epoch === inboxEpoch) renderNotifications()
}
notificationsRead.addEventListener('click', () => void readSelectedNotifications())

async function openNotification(item) {
  if (notificationBusy) return
  notificationBusy = true
  const epoch = inboxEpoch
  // The web inbox acknowledges in the background; opening the destination should not wait on the network.
  const read = invoke('daemon_request', { method: 'notifications_read', params: { id: item.id } })
    .then(next => { if (epoch === inboxEpoch) receiveNotifications(next) })
    .catch(error => { if (epoch === inboxEpoch) { notificationError = errorText(error); renderNotifications() } })
  try {
    if (epoch === inboxEpoch) {
      await invoke('open_app', { path: item.href })
      statsOpen = false
      wake()
    }
  } catch (error) { if (epoch === inboxEpoch) { notificationError = errorText(error); renderNotifications() } }
  await read
  notificationBusy = false
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
    icon.alt = emoteLabel(emote)
    friendsNote.append(icon)
  }
  friendsNote.append(text)
}

async function loadFriends() {
  const current = ++friendsRequest
  friends = []
  friendsPageIndex = 0
  renderFriends()
  friendsOpenApp.hidden = true
  note('Loading friends…')
  let rows
  try {
    const result = await invoke('daemon_request', { method: 'friends', params: {} })
    rows = friendRows(result)
    pins = Array.isArray(result?.pins) && result.pins.length === 3 && result.pins.every(key => typeof key === 'string' && EMOTE_KEY.test(key))
      ? result.pins : [...VISIT_EMOTES]
  }
  catch (error) {
    if (current === friendsRequest) note(errorText(error), { error: true })
    wake()
    return
  }
  if (current !== friendsRequest) return
  note(rows.length ? null : 'No friends yet — add some on Clash of Meat')
  friendsOpenApp.hidden = rows.length > 0
  friends = rows
  renderFriends()
  wake()
}

// One page of the list at a time; the pager only shows when there is more than one page.
function renderFriends() {
  const { rows, page, pages } = friendPage(friends, friendsPageIndex)
  friendsPageIndex = page
  friendsList.replaceChildren(...rows.map(friendRow))
  friendsPager.hidden = pages < 2
  friendsPageLabel.textContent = `${page + 1} / ${pages}`
  friendsPrev.disabled = page === 0
  friendsNext.disabled = page >= pages - 1
}

function turnFriendsPage(step) {
  if (sending) return
  friendsPageIndex += step
  renderFriends()
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
    for (const name of pins) {
      const button = emoteButton(name)
      button.title = `${emoteLabel(name)} to ${friend.name}`
      button.addEventListener('click', () => name === SIGN_EMOTE ? openSignStep(friend) : void sendVisit(friend, name))
      row.append(button)
    }
  }
  return row
}

// Review me asks for the link before it goes: one more step in the same bubble, which takes the keyboard.
function openSignStep(friend) {
  if (sending) return
  signTarget = friend
  document.getElementById('sign-title').textContent = `Review me · ${friend.name}`
  signUrl.value = ''
  checkSignUrl()
  friendsPanel.hidden = true
  signStep.hidden = false
  wake()
}

function checkSignUrl() {
  const raw = signUrl.value.trim()
  const result = checkReviewLink(raw)
  const state = !raw ? '' : result.ok ? 'good' : 'bad'
  signUrl.className = state
  signWhy.className = `sign-why ${state}`
  signWhy.textContent = result.ok ? `${signTarget?.name ?? 'Your friend'} gets a sign on their desktop.` : result.why
  signSend.disabled = !result.ok || sending
  return result.ok
}

async function sendSign() {
  if (!signTarget || sending || !checkSignUrl()) return
  const sent = await sendVisit(signTarget, SIGN_EMOTE, { url: signUrl.value.trim() })
  if (!sent) checkSignUrl()
}

// The hero window takes the keyboard only while the link step is open on screen (overlay.rs hero_keyboard).
function syncKeyboard(wanted) {
  if (wanted === keyboard) return
  keyboard = wanted
  void invoke('hero_keyboard', { on: wanted })
    .then(() => { if (wanted) signUrl.focus() })
    .catch(error => console.error('hero keyboard:', error))
}

async function sendVisit(friend, emote, sign) {
  if (sending) return false
  sending = true
  const buttons = [...friendsList.querySelectorAll('button'), signSend]
  for (const button of buttons) button.disabled = true
  let sent = false
  try {
    const result = await invoke('daemon_request', { method: 'send_visit', params: { target: friend.handle, emote, ...(sign ? { sign } : {}) } })
    if (!parseOutgoingVisit(result)) throw new Error('The visit was sent without a valid visit ID.')
    outgoing.identify(player?.handle)
    outgoing.sent(result)
    if (outgoing.away() && character && animation && workArea) {
      const now = Date.now()
      const pose = activity.snapshot(now, { width: Math.max(0, workArea.width - HERO_CELL_SIZE), still: true })
      departure = beginDeparture({ x: pose.x, width: Math.max(0, workArea.width - HERO_CELL_SIZE), facing: pose.facing, startedAt: now })
    }
    note(null)
    statsOpen = false
    menuOpen = false
    showFriends(false)
    sent = true
  } catch (error) {
    if (signStep.hidden) note(errorText(error), { error: true })
    else { signWhy.className = 'sign-why bad'; signWhy.textContent = errorText(error) }
  }
  for (const button of buttons) button.disabled = false
  sending = false
  wake()
  return sent
}

// The daemon acknowledges it with the next heartbeat, and the server stops re-sending it.
function visitDone(id) {
  void invoke('daemon_request', { method: 'visit_done', params: { id } }).catch(error => console.error('visit done:', error))
}

function setStayStill(next) {
  if (next && !stayStill && workArea) {
    const now = Date.now()
    activity.drag(activity.snapshot(now, { width: Math.max(0, workArea.width - HERO_CELL_SIZE), still: true }).x, now)
  }
  if (!next && stayStill && workArea) activity.strollNow(Date.now())
  stayStill = next
  stillButton.textContent = stayStill ? 'Start walking' : 'Stop walking'
  wake()
}

async function setHero(data, heartbeat = false) {
  let now = Date.now()
  if (data && Object.hasOwn(data, 'player')) outgoing.identify(data.player?.handle, now)
  if (data && Object.hasOwn(data, 'outgoingVisit')) outgoing.heartbeat(data.outgoingVisit, now, heartbeat)
  const candidate = validateCharacter(data?.character)
  const nextAnimation = validateAnimation(data?.animation ?? candidate?.animation)
  const nextAura = validateLevelUp(data?.levelUp ?? data?.level_up)
  const current = ++generation
  // A switched hero brings over 1 MB of new art. Decode it before showing any of it, so the old
  // hero stays whole until the new one swaps in at once: never a new name on an old skin, or
  // new frames that draw nothing yet. A newer heartbeat that arrives meanwhile wins.
  if (candidate && nextAnimation) {
    try { await Promise.all(heroPngs(candidate, nextAnimation, nextAura).map(heroImage)) }
    catch (error) { console.error('hero image decoding:', error); return }
    if (current !== generation) return
    now = Date.now()
  }
  const previous = character
  character = candidate
  animation = nextAnimation
  progression = data?.progression ?? null
  player = data?.player ?? null
  effectImage(`./assets/flex-fx/${leagueOrBronze(player?.league)}.png`)
  effectImage('./assets/emote-kiss.png')
  effectImage('./assets/angry-mark.png')
  // A later heartbeat carries no effect; let a playing aura finish, unless it shows a hero just switched away.
  if (nextAura || !(aura && now - auraStarted < aura.durationMs) || previous?.id !== character?.id) aura = nextAura
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
      // With friends' signs turned off in Settings, a Review me visitor just blows a kiss.
      visitor = visit.emote === SIGN_EMOTE && !allowSigns ? { ...visit, emote: 'kiss', sign: undefined } : visit
      visitorStarted = now
      visitSpot = null
      visitLabel.textContent = visit.name
    } else if (visitor?.id !== visit.id) visitDone(visit.id)
  }
  if (character && animation) {
    // The stats change with the skin, not after the visitor's frames or the geometry below.
    updateStats()
    try { await decodeHero() } catch (error) { console.error('hero image decoding:', error); return }
    if (current !== generation) return
    // The previous skin's images can no longer be drawn; each switch would otherwise keep them.
    if (previous && previous.png !== character.png) {
      const kept = new Set(heroPngs())
      for (const png of images.keys()) if (!kept.has(png)) images.delete(png)
    }
    if (!workArea) {
      try { ({ work_area: workArea } = await invoke('hero_geometry', { screen })) }
      catch (error) { console.error('hero geometry:', error); return }
    }
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

function drawHeroImage(png, x, y, frameSize, flip = false, opacity = 1, squash) {
  const place = heroDrawRect(x, y, frameSize)
  if (!squash) return drawImage(png, place.x, place.y, place.width, flip, opacity, place.height)
  // Squashed and stretched about the feet (Review me's plant): scaled around the bottom centre of the frame.
  ctx.save()
  ctx.translate(place.x + place.width / 2, place.y + place.height)
  ctx.scale(squash.sx, squash.sy)
  drawImage(png, -place.width / 2, -place.height, place.width, flip, opacity, place.height)
  ctx.restore()
}

function drawEmoteEffect(state, elapsed, frame, spriteX, spriteY, facing, league, leagueRank, hostSpriteX = null) {
  if (state === 'flex') {
    const phase = flexPhase(elapsed)
    if (phase < 0) return
    const sheet = effectImage(`./assets/flex-fx/${leagueOrBronze(league)}.png`)
    if (!sheet) return
    const place = hostSpriteX === null
      ? flexPlace(spriteX, spriteY, rect.width)
      : visitorFlexPlace(spriteX, spriteY, leagueOrBronze(league))
    const width = place.width ?? FLEX_WIDTH
    const height = place.height ?? FLEX_HEIGHT
    ctx.drawImage(sheet, 0, phase * FLEX_HEIGHT, FLEX_WIDTH, FLEX_HEIGHT, place.x, place.y, width, height)
    const label = flexLabel(leagueRank)
    if (phase >= 1 && phase <= 9 && label) {
      ctx.save()
      ctx.font = '18px Righteous, Georgia, serif'
      ctx.textAlign = 'center'
      ctx.textBaseline = 'alphabetic'
      ctx.lineJoin = 'round'
      ctx.lineWidth = 4
      ctx.strokeStyle = '#241a14'
      ctx.fillStyle = '#fff1cc'
      const center = place.x + width / 2
      const textWidth = flexLabelMaxWidth(center, rect.width)
      const baseline = place.y + (leagueOrBronze(league) === 'bronze' ? 153 : 172) * (height / FLEX_HEIGHT)
      ctx.strokeText(label, center, baseline, textWidth)
      ctx.fillText(label, center, baseline, textWidth)
      ctx.restore()
    }
  } else if (state === 'kiss' && frame === 8) {
    const heart = effectImage('./assets/emote-kiss.png')
    if (!heart) return
    const place = hostSpriteX === null
      ? kissHeartPlace(spriteX, spriteY, rect.width, facing, elapsed)
      : visitorKissHeartPlace(hostSpriteX, spriteX, spriteY, rect.width, elapsed)
    if (!place) return
    ctx.drawImage(heart, place.x, place.y, place.size, place.size)
  } else if (state === 'angry' && frame === 12) {
    const mark = effectImage('./assets/angry-mark.png')
    if (!mark) return
    const pulse = 1 + 0.08 * Math.sin(elapsed / 85)
    const size = 57 * pulse
    const centerX = spriteX + (facing === 'left' ? 35 : 93)
    const x = Math.max(0, Math.min(rect.width - size, centerX - size / 2))
    // Half the old 25px gap above the sprite box, still clear of the head.
    ctx.drawImage(mark, x, spriteY - 12 - size / 2, size, size)
  }
}

function deadline(now, pose) {
  if (fall) return 16
  const delays = []
  const clip = frameDelay(animation, pose.state, pose.elapsed)
  if (Number.isFinite(clip)) delays.push(clip)
  if (Number.isFinite(pose.until)) delays.push(pose.until)
  if (bellRingUntil > Date.now()) delays.push(bellRingUntil - Date.now())
  if (pose.state === 'walk' && !paused) delays.push(83)
  if (pose.state === 'flex') delays.push(flexDelay(pose.elapsed))
  if (pose.state === 'kiss' || pose.state === 'angry') delays.push(83)
  // The plant squashes and hops: draw it smoothly while it plays.
  if (visitor?.emote === SIGN_EMOTE && now - visitorStarted >= WALK_IN && now - visitorStarted < visitDuration(visitor.character.animation, SIGN_EMOTE)) delays.push(33)
  if (gained && bubbleFrame(gained, now).visible) delays.push(83, Math.max(0, gained.startedAt + 1800 - now))
  if (aura && now - auraStarted < aura.durationMs) delays.push(83, aura.durationMs - (now - auraStarted))
  if (visitor && now - visitorStarted < visitDuration(visitor.character.animation, visitor.emote)) {
    delays.push(83, visitDuration(visitor.character.animation, visitor.emote) - (now - visitorStarted))
    const visitorFlexElapsed = now - visitorStarted - 1500
    if (visitor.emote === 'flex' && visitorFlexElapsed >= 0 && visitorFlexElapsed < FLEX_DURATION) delays.push(flexDelay(visitorFlexElapsed))
  }
  return delays.length ? Math.max(16, Math.min(...delays)) : Infinity
}

async function draw() {
  timer = undefined
  if (!character || !animation || !workArea || hidden || paused) {
    notificationBell.hidden = true
    syncKeyboard(false)
    return
  }
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
  if (visitor && now - visitorStarted >= visitDuration(visitor.character.animation, visitor.emote)) {
    visitDone(visitor.id)
    visitor = null
  }
  // A carried hero takes its visitor along; the visit is pinned again where it is let go.
  if (!visitor || drag?.moved) visitSpot = null
  let away = outgoing.away(now)
  if (!away) departure = null
  let leaving = away && departure ? departurePose(departure, now) : null
  if (leaving?.done) { departure = null; leaving = null }
  const patrolWidth = Math.max(0, workArea.width - HERO_CELL_SIZE)
  const pose = activity.snapshot(now, { width: patrolWidth, still: stayStill || statsOpen || menuOpen || away || Boolean(visitor) })
  // One fixed size: the window only moves, so opening the card never resizes it (no flash).
  const boundedX = Math.max(0, Math.min(patrolWidth, leaving?.x ?? pose.x))
  const heroRect = windowRect(boundedX, { workArea, bubble: true, lift })
  if (visitor && !visitSpot && !drag?.moved) {
    // The host stops where it is to greet the visitor, and the window stays here until the visit ends.
    if (!leaving) activity.drag(pose.x, now)
    visitSpot = visitAnchor(heroRect)
  }
  const nextRect = visitSpot ? anchoredRect(heroRect, visitSpot) : heroRect
  const changed = !lastPlaced || Math.abs(nextRect.x - lastPlaced.x) >= 1 || nextRect.y !== lastPlaced.y || nextRect.width !== lastPlaced.width || nextRect.height !== lastPlaced.height
  rect = nextRect
  if (changed) {
    lastPlaced = nextRect
    try { await invoke('hero_place', { x: rect.x, y: rect.y, width: rect.width, height: rect.height }) }
    catch (error) { console.error('hero placement:', error); return }
  }
  away = outgoing.away(Date.now())
  if (away && !leaving && !visitor) {
    stats.hidden = true
    menu.hidden = true
    syncKeyboard(false)
    notificationBell.hidden = true
    visitLabel.hidden = true
    updateHitRegions()
    if (visible) { await nativeWindow.hide(); visible = false }
    if (!outgoing.away() || visitor) wake()
    return
  }
  if (!visible) { await nativeWindow.show(); visible = true }
  away = outgoing.away(Date.now())
  if (away && !leaving && !visitor) {
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
    const size = LEGACY_FRAME_SIZE * (aura.renderScale ?? 1)
    drawImage(aura.frames[levelUpFrame(aura, now - auraStarted)], rect.spriteX + HERO_CELL_SIZE / 2 - size / 2, rect.spriteY + HERO_CELL_SIZE - size, size, pose.facing === 'left')
  } else if (!away || leaving) {
    const active = leaving ?? pose
    const frame = companionFrame(animation, active.state, active.elapsed)
    const chosen = frameImage(animation, frame, active.facing)
    const motion = active.state === 'flex' ? flexMotion(active.elapsed) : { dx: 0, dy: 0 }
    const exitOffset = leaving ? leaving.x - boundedX : 0
    const hostX = rect.spriteX + exitOffset
    // In a window pinned for a visitor, a host leaving the window fades at its edge.
    const opacity = (leaving?.opacity ?? 1) * (visitSpot ? edgeFade(hostX, rect.width) : 1)
    drawHeroImage(chosen.png ?? character.png, hostX + motion.dx, rect.spriteY + motion.dy, animation.size, chosen.flip, opacity)
    if (!leaving) {
      const effect = heroEffectAnchor(rect.spriteX, rect.spriteY)
      drawEmoteEffect(active.state, active.elapsed, frame, effect.x, effect.y, active.facing, player?.league, player?.leagueRank)
    }
  }
  visitLabel.hidden = true
  if (visitor) {
    const visitAnimation = visitor.character.animation
    // Drawn from where the host stood when the visitor arrived, not from where the host is now.
    const place = visitSpot ?? visitAnchor(rect)
    const visitorX = place.spriteX
    const v = visitPose(visitAnimation, now - visitorStarted, place.stand, place.edge, visitor.emote)
    const frame = frameImage(visitAnimation, v.frame, v.facing)
    const visitElapsed = now - visitorStarted - 1500
    const motion = v.state === 'flex' ? flexMotion(visitElapsed) : { dx: 0, dy: 0 }
    const visitorCellX = visitorX + v.offset
    const squash = v.state === SIGN_EMOTE ? { sx: v.sx, sy: v.sy } : undefined
    drawHeroImage(frame.png, visitorCellX + motion.dx, rect.spriteY + motion.dy + (v.dy ?? 0), visitAnimation.size, frame.flip, 1, squash)
    // The sign drops beside the visitor, away from the host, into the signs window (signs.mjs) that keeps it.
    if (v.state === SIGN_EMOTE && visitor.sign && !visitor.planted && now - visitorStarted - WALK_IN >= SIGN_DROP_AT) {
      visitor.planted = true
      const outward = v.offset > 0 ? 1 : -1
      const x = signPostX(rect.x + visitorCellX + HERO_CELL_SIZE / 2, outward, workArea)
      void emit('sign-plant', { id: visitor.id, url: visitor.sign.url, sender: visitor.name, x, area: workArea })
    }
    // Both contract versions send the bare Kiss frame for visitors; draw its one shared
    // heart between the two heroes.
    const visitorEffect = heroEffectAnchor(visitorCellX, rect.spriteY)
    const hostEffect = heroEffectAnchor(visitorX, rect.spriteY)
    drawEmoteEffect(v.state, visitElapsed, v.frame, visitorEffect.x, visitorEffect.y, v.facing, visitor.league, visitor.leagueRank, hostEffect.x)
    // Measured while shown: a hidden label is 0 px wide, which put its left edge on the hero's centre.
    visitLabel.hidden = false
    const left = visitLabelLeft(visitorCellX, visitLabel.offsetWidth, rect.width)
    visitLabel.hidden = left === null || !visitLabelVisible(now - visitorStarted, visitDuration(visitAnimation, visitor.emote))
    visitLabel.style.left = `${left ?? 0}px`
    visitLabel.style.top = `${visitorEffect.y - 4}px`
  }
  if (!away && gained) {
    // Rises 32 px above the head, as in the terminal.
    ctx.save()
    ctx.translate(0, rect.spriteY + HERO_CELL_SIZE - LEGACY_FRAME_SIZE - 30)
    drawXpText(ctx, xpTextFrame(gained.amount, now - gained.startedAt, { centerX: rect.spriteX + HERO_CELL_SIZE / 2, areaHeight: 46, scale: 2, maxWidth: rect.width }))
    ctx.restore()
  }
  if (!away && character.canLevelUp === true && !evolving && !auraPlaying) void evolve()
  stats.hidden = away || !statsOpen || menuOpen
  menu.hidden = away || !menuOpen
  syncKeyboard(!stats.hidden && !signStep.hidden)
  document.documentElement.style.setProperty('--hero-center', `${rect.spriteX + HERO_CELL_SIZE / 2}px`)
  showBell(away)
  if (!notificationBell.hidden) {
    const at = bellPosition(rect)
    notificationBell.style.left = `${at.left}px`
    notificationBell.style.top = `${at.top}px`
  }
  updateHitRegions()
  const next = deadline(now, leaving ?? pose)
  if (Number.isFinite(next)) timer = setTimeout(draw, next)
}

// Only these rectangles take clicks; the rest of the window lets them through (overlay.rs).
function updateHitRegions() {
  const regions = outgoing.away() ? [] : drag ? [[0, 0, rect.width, rect.height]] : [[rect.spriteX + 16, rect.spriteY + 32, 160, 160]]
  for (const element of [notificationBell, stats, menu]) {
    if (element.hidden) continue
    const box = element.getBoundingClientRect()
    regions.push([box.left, box.top, box.width, box.height])
  }
  const key = JSON.stringify(regions)
  if (key === hitKey) return
  hitKey = key
  void invoke('hero_hit', { regions }).catch(error => console.error('hero hit regions:', error))
}

// Plays one of the contract's clips once the hero's art has arrived.
function play(name, at, durationMs) {
  if (animation) activity.emote(name, at, durationMs)
}

function wake() {
  if (timer) clearTimeout(timer)
  timer = undefined
  void draw()
}

canvas.addEventListener('pointerdown', event => {
  if (!rect || !character || outgoing.away() || event.button !== 0) return
  const x = event.clientX, y = event.clientY
  if (x < rect.spriteX || x > rect.spriteX + HERO_CELL_SIZE || y < rect.spriteY || y > rect.spriteY + HERO_CELL_SIZE) return
  // Grabbing a falling hero catches it where it is.
  fall = null
  drag = { startX: event.screenX, startY: event.screenY, startLift: lift, starting: activity.snapshot(Date.now(), { width: workArea.width - HERO_CELL_SIZE, still: true }).x, moved: false }
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
    lift = Math.max(0, Math.min(workArea.height - HERO_CELL_SIZE, drag.startLift + drag.startY - event.screenY))
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
  else play('grab', Date.now(), 0)
  // A hero let go in the air falls back to the ground, also one caught mid-fall by a click.
  if (lift >= 2) fall = { from: lift, startedAt: Date.now() }
  drag = null
  wake()
}
canvas.addEventListener('pointerup', release)
canvas.addEventListener('pointercancel', release)
canvas.addEventListener('contextmenu', event => {
  event.preventDefault()
  if (outgoing.away()) return
  if (!rect || event.clientX < rect.spriteX || event.clientX > rect.spriteX + HERO_CELL_SIZE || event.clientY < rect.spriteY || event.clientY > rect.spriteY + HERO_CELL_SIZE) return
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
document.getElementById('sign-back').addEventListener('click', () => { signStep.hidden = true; friendsPanel.hidden = false; wake() })
signUrl.addEventListener('input', checkSignUrl)
signUrl.addEventListener('keydown', event => {
  if (event.key === 'Enter') void sendSign()
  if (event.key === 'Escape') { signStep.hidden = true; friendsPanel.hidden = false; wake() }
})
signSend.addEventListener('click', () => void sendSign())
// Pulls out every friend's sign at once (signs.mjs).
removeSigns.addEventListener('click', () => { void emit('signs-clear') })
function showSignCount(count) {
  signCount = Number.isSafeInteger(count) && count > 0 ? count : 0
  removeSigns.hidden = signCount === 0
  document.getElementById('sign-count').textContent = `(${signCount})`
  wake()
}
notificationBell.addEventListener('pointerdown', event => event.stopPropagation())
notificationBell.addEventListener('click', () => {
  if (outgoing.away()) return
  const closing = statsOpen && !notificationsPanel.hidden
  statsOpen = !closing
  menuOpen = false
  showNotifications(!closing)
  wake()
})
// The hero card's bell swaps the card for the inbox in the same bubble; its back arrow returns to the card.
cardBell.addEventListener('click', () => { showNotifications(true); wake() })
document.getElementById('notifications-back').addEventListener('click', () => { showNotifications(false); wake() })
document.getElementById('notifications-all').addEventListener('click', async () => {
  try { await invoke('open_app', { path: '/notifications' }); statsOpen = false; wake() }
  catch (error) { notificationError = errorText(error); renderNotifications() }
})
friendsPrev.addEventListener('click', () => turnFriendsPage(-1))
friendsNext.addEventListener('click', () => turnFriendsPage(1))
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
document.getElementById('feedback').addEventListener('click', async () => {
  menuOpen = false
  wake()
  await emit('overlay-control', { action: 'feedback' })
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
  if (next !== paused && workArea) activity.snapshot(Date.now(), { width: Math.max(0, workArea.width - HERO_CELL_SIZE), paused: true, still: true })
  paused = next
}
window.addEventListener('resize', wake)
let checkingOutgoing = false, outgoingChecks = 0
setInterval(async () => {
  const now = Date.now()
  if (outgoing.expire(now)) wake()
  if (checkingOutgoing) return
  const ids = outgoing.ids(now)
  if (!ids.length) { outgoingChecks = 0; return }
  // Every 3 s for the first 20 s, when most visits are played, then every 12 s: heartbeats
  // and the visit's own expiry bring the hero back anyway.
  if (outgoingChecks++ > 6 && outgoingChecks % 4) return
  const owner = outgoing.owner()
  checkingOutgoing = true
  try {
    for (const id of ids) {
      try {
        const result = await invoke('daemon_request', { method: 'visit_status', params: { id } })
        if (owner === outgoing.owner() && outgoing.status(result?.visit)) wake()
      } catch { /* An offline sender stays away until the visit expires locally. */ }
    }
  } finally { checkingOutgoing = false }
}, 3000)
// Display selection can change under us when a monitor disconnects or its work area moves.
// hero_geometry falls back to the primary display if the selected display is gone.
setInterval(async () => {
  if (paused) return
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
    localPending = payload.data?.localPending ?? null
    localToday = payload.data?.localToday ?? null
    normaliseActivity(payload.data?.activity)
    notificationHeartbeat(payload.data?.hero)
    void setHero(payload.data?.hero)
  } else if (payload?.kind === 'event') {
    if (payload.data?.event === 'activity') normaliseActivity(payload.data.data)
    if (payload.data?.event === 'heartbeat') {
      localPending = payload.data.data?.localPending ?? null
      localToday = payload.data.data?.localToday ?? localToday
      notificationHeartbeat(payload.data.data)
      void setHero(payload.data.data, true)
    }
    if (payload.data?.event === 'local_tokens') {
      localPending = payload.data.data?.pending ?? null
      localToday = payload.data.data?.today ?? localToday
      updateStats()
    }
  } else if (payload?.kind === 'status' && payload.data?.connected === false) {
    inboxEpoch++
    inbox = null
    inboxOwner = null
    notificationError = ''
    renderNotifications()
    normaliseActivity({ busy: false, sessions: [] })
  }
})
const initial = await invoke('desktop_state')
const initialHero = heroSettings(initial.hero)
screen = initialHero.screen
setStayStill(initialHero.still)
allowSigns = initialHero.signs
setPaused(!initialHero.show)
const snapshot = initial.daemon?.snapshot
if (snapshot) {
  localPending = snapshot.localPending ?? null
  localToday = snapshot.localToday ?? null
}
if (snapshot) normaliseActivity(snapshot.activity)
const firstHero = initial.daemon?.heartbeat ?? snapshot?.hero
if (firstHero) { notificationHeartbeat(firstHero); void setHero(firstHero) }
// A cached snapshot may predate the visit. Refresh it instead of waiting for the idle minute.
if (initial.daemon?.connected) void invoke('daemon_request', { method: 'sync_now', params: {} }).catch(() => {})
await listen('hero-settings', ({ payload }) => {
  const next = heroSettings(payload)
  setStayStill(next.still)
  // Turning friends' signs off also takes down the ones already up.
  if (allowSigns && !next.signs) void emit('signs-clear')
  allowSigns = next.signs
  const nextScreen = next.screen
  if (screen !== nextScreen) { screen = nextScreen; workArea = undefined; lastPlaced = undefined; void invoke('hero_geometry', { screen }).then(value => { workArea = value.work_area; wake() }) }
  setPaused(!next.show)
  if (paused) { if (timer) clearTimeout(timer); timer = undefined; void nativeWindow.hide(); visible = false }
  else { hidden = false; wake() }
})
// A click elsewhere on screen near the hero (overlay.rs reports it in window coordinates) sends it
// hurrying away, so it never sits on what the user is reaching for. "Stop walking" keeps it put.
await listen('screen-click', ({ payload }) => {
  // overlay.rs reports only clicks outside the hit regions, so any click away from an open card or menu closes it.
  if (statsOpen || menuOpen) {
    statsOpen = false
    menuOpen = false
    wake()
    return
  }
  if (!rect || !character || outgoing.away() || paused || hidden || stayStill || drag || fall || visitor) return
  if (aura && Date.now() - auraStarted < aura.durationMs) return
  const dx = payload.x - (rect.spriteX + HERO_CELL_SIZE / 2), dy = payload.y - (rect.spriteY + HERO_CELL_SIZE - 64)
  if (Math.hypot(dx, dy) > 260) return
  activity.flee(dx, Date.now())
  wake()
})
await listen('signs-count', ({ payload }) => showSignCount(payload?.count))
void emit('signs-ask')
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
