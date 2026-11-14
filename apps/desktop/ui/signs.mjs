import { stripRect, signBox, signRegion, signTilt, slept, plantRequest, sameArea, MAX_SIGNS, STRIP_HEIGHT } from './signs/board.mjs'
import { linkLabel } from './overlay/review-link.mjs'

// Friends' Review me signs. A visitor plants one beside it (overlay.mjs sends `sign-plant`); it stays for this
// session: hovering shows who sent it, clicking opens the link, × pulls it out. Remove all (the hero card), quitting
// the app or the computer sleeping clears them all. Nothing is stored. The window lets clicks through except over
// a sign (overlay.rs).
const { invoke } = window.__TAURI__.core
const { listen, emit } = window.__TAURI__.event

for (const type of ['selectstart', 'dragstart', 'contextmenu']) document.addEventListener(type, event => event.preventDefault())

let area = null
const signs = []

function element(tag, className, text) {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text) node.textContent = text
  return node
}

function signNode(sign) {
  const node = element('div', 'sign drop')
  node.style.setProperty('--tilt', `${sign.tilt.toFixed(1)}deg`)
  const art = element('div', 'art')
  const img = element('img')
  img.src = './assets/sign-review-me.png'
  img.alt = ''
  art.append(img)
  const board = element('div', 'board')
  board.append(element('span', '', 'Review me'))
  art.append(board)
  const hit = element('div', 'hit')
  const close = element('button', 'close', '×')
  close.title = 'Remove this sign'
  close.setAttribute('aria-label', `Remove ${sign.sender}'s sign`)
  const card = element('div', 'card')
  const who = element('div', 'who')
  who.append(element('small', '', 'From'), sign.sender)
  const { host, rest } = linkLabel(sign.url)
  const link = element('span', 'link')
  link.append(element('b', '', host), rest)
  card.append(who, link)
  node.append(art, hit, close, card)
  node.setAttribute('role', 'link')
  node.setAttribute('aria-label', `Review me from ${sign.sender}: ${sign.url}`)
  hit.addEventListener('click', () => void invoke('open_review_link', { link: sign.url }).catch(error => console.error('open review link:', error)))
  close.addEventListener('click', event => { event.stopPropagation(); remove(sign, true) })
  node.addEventListener('animationend', () => node.classList.remove('drop'), { once: true })
  return node
}

function place(sign) {
  const box = signBox(sign.x, area)
  sign.box = box
  sign.node.style.left = `${box.left}px`
  sign.node.style.top = `${box.top}px`
}

function dust(sign) {
  const foot = sign.box.left + 78
  for (const [dx, offset] of [[-14, -16], [14, 10]]) {
    const puff = element('div', 'dust')
    puff.style.left = `${foot + offset}px`
    puff.style.top = `${STRIP_HEIGHT - 10}px`
    puff.style.setProperty('--dx', `${dx}px`)
    puff.addEventListener('animationend', () => puff.remove(), { once: true })
    document.body.append(puff)
  }
}

// The strip follows the screen the newest sign was planted on; signs already up keep their place on it.
async function useArea(next) {
  if (sameArea(area, next)) return
  area = next
  const strip = stripRect(area)
  try { await invoke('hero_place', strip) } catch (error) { console.error('signs placement:', error) }
  for (const sign of signs) place(sign)
}

async function plant(value) {
  const request = plantRequest(value)
  if (!request || signs.some(sign => sign.id === request.id)) return
  await useArea(request.area)
  while (signs.length >= MAX_SIGNS) remove(signs[0], false)
  const sign = { ...request, tilt: signTilt() }
  sign.node = signNode(sign)
  place(sign)
  signs.push(sign)
  // Newest on top: later nodes paint over earlier ones.
  document.body.append(sign.node)
  // The drop lands 300 ms in, on the visitor's stomp.
  setTimeout(() => { if (signs.includes(sign)) dust(sign) }, 300)
  changed()
}

function remove(sign, animated) {
  const index = signs.indexOf(sign)
  if (index < 0) return
  signs.splice(index, 1)
  if (animated) {
    sign.node.classList.remove('drop')
    sign.node.classList.add('pull')
    sign.node.addEventListener('animationend', () => sign.node.remove(), { once: true })
  } else sign.node.remove()
  changed()
}

function clearAll(animated) {
  for (const sign of [...signs]) remove(sign, animated)
}

let hitKey = ''
function changed() {
  const regions = signs.map(sign => signRegion(sign.box))
  const key = JSON.stringify(regions)
  if (key !== hitKey) {
    hitKey = key
    void invoke('hero_hit', { regions }).catch(error => console.error('signs hit regions:', error))
  }
  void emit('signs-count', { count: signs.length })
}

// Signs last for the session the computer is awake: a long gap between ticks means it slept.
let tick = Date.now()
setInterval(() => {
  const now = Date.now()
  if (slept(tick, now)) clearAll(false)
  tick = now
}, 5000)

await listen('sign-plant', ({ payload }) => void plant(payload))
await listen('signs-clear', () => clearAll(true))
// The hero card asks once it loads, in case signs went up before it.
await listen('signs-ask', () => void emit('signs-count', { count: signs.length }))
