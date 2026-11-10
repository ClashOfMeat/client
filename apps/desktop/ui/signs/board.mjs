import { checkReviewLink } from '../overlay/review-link.mjs'

// The signs window is a strip along the bottom of the hero's screen, tall enough for a sign and its hover card.
export const STRIP_HEIGHT = 240
// One sign: 156×144 px (104×96 art at 1.5), its post at the middle, its foot 131 px down.
export const SIGN_WIDTH = 156
export const SIGN_FOOT = 131
// Older signs make way for new ones; the hero window takes up to 8 hit regions, this one 12 (overlay.rs).
export const MAX_SIGNS = 12
// A gap this long between two ticks of a 5 s timer means the computer slept: its signs are cleared.
export const SLEEP_GAP_MS = 60_000

export function stripRect(area) {
  return { x: area.x, y: area.y + area.height - STRIP_HEIGHT, width: area.width, height: STRIP_HEIGHT }
}

// The sign's box in the strip, from its post's x on screen.
export function signBox(postX, area) {
  return { left: Math.round(postX - area.x - SIGN_WIDTH / 2), top: STRIP_HEIGHT - SIGN_FOOT }
}

// Where the sign takes the pointer: its board, post and close button, with room for its lean.
export function signRegion(box) {
  return [box.left + 22, box.top, 112, SIGN_FOOT + 6]
}

// 2 to 5 degrees either way, so a row of signs never looks stamped.
export function signTilt(random = Math.random) {
  return (random() < 0.5 ? -1 : 1) * (2 + 3 * random())
}

export const slept = (previousTick, now, gap = SLEEP_GAP_MS) => now - previousTick > gap

const finite = n => typeof n === 'number' && Number.isFinite(n)

// A plant the hero window asked for: { id, url, sender, x, area }, or null. The link is checked again here,
// since this window opens it.
export function plantRequest(value) {
  if (!value || typeof value !== 'object' || typeof value.id !== 'string' || !value.id || value.id.length > 80) return null
  if (!checkReviewLink(value.url).ok || typeof value.sender !== 'string' || !finite(value.x)) return null
  const area = value.area
  if (!area || ![area.x, area.y, area.width, area.height].every(finite) || area.width < 1 || area.height < 1) return null
  const sender = value.sender.replace(/[\x00-\x1f\x7f]/g, '').slice(0, 80) || 'A friend'
  return { id: value.id, url: value.url, sender, x: value.x, area: { x: area.x, y: area.y, width: area.width, height: area.height } }
}

export const sameArea = (a, b) => Boolean(a && b) && a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height
