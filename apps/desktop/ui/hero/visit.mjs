import { visitPlace } from './effects.mjs'
import { HERO_CELL_SIZE } from './display.mjs'

// The server re-sends a visit on every heartbeat until it is acknowledged, so each one plays once.
export function visitTracker(limit = 50) {
  const seen = []
  return {
    // True the first time this visit ID is offered.
    first(id) {
      if (seen.includes(id)) return false
      seen.push(id)
      if (seen.length > limit) seen.shift()
      return true
    },
  }
}

// Left edge of the player-name label: centred over the visitor and kept inside the window,
// or null while the visitor is outside it.
export function visitLabelLeft(visitorX, labelWidth, windowWidth, sprite = HERO_CELL_SIZE) {
  const center = visitorX + sprite / 2
  if (center < 0 || center > windowWidth) return null
  return Math.round(Math.max(2, Math.min(windowWidth - labelWidth - 2, center - labelWidth / 2)))
}

// Where a visit plays on screen, fixed when the visitor arrives: the window's screen x, the host's
// place in it then, and the visitor's stand and entry edge beside that place. The visitor keeps to
// it even if the host then walks off (a visit sent back at that moment), instead of being carried along.
export function visitAnchor(rect) {
  return { x: rect.x, spriteX: rect.spriteX, ...visitPlace(rect.spriteX, rect.width) }
}

// The host's window while a visitor is on screen: pinned at the anchor, with the host sprite
// moved inside it to where the host really is.
export function anchoredRect(rect, anchor) {
  return { ...rect, x: anchor.x, spriteX: rect.spriteX + rect.x - anchor.x }
}

// A host walking out of the pinned window fades as it crosses the edge instead of being cut off:
// fully drawn while inside, gone once half of it is out.
export function edgeFade(spriteX, windowWidth, sprite = HERO_CELL_SIZE) {
  const inside = Math.max(0, Math.min(spriteX + sprite, windowWidth) - Math.max(spriteX, 0)) / sprite
  return Math.max(0, Math.min(1, inside * 2 - 1))
}
