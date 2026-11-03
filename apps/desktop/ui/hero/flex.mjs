// The hero has four poses: start for 300ms, two moving poses for 125ms each,
// then a still final pose for 3000ms. The twelve-frame crest spans that clip.
export const FLEX_DELAYS = [300, 250, 100, 100, 100, 100, 100, 100, 100, 2150, 100, 50]
export const FLEX_DURATION = FLEX_DELAYS.reduce((sum, delay) => sum + delay, 0)
export const FLEX_WIDTH = 180
export const FLEX_HEIGHT = 200
export const VISITOR_FLEX_SCALE = 0.8
const NORMAL = { dx: 0, dy: 0, width: 128, height: 128 }

export function flexPhase(elapsed) {
  if (!Number.isFinite(elapsed) || elapsed < 0 || elapsed >= FLEX_DURATION) return -1
  let remaining = elapsed
  for (let phase = 0; phase < FLEX_DELAYS.length; phase++) {
    if (remaining < FLEX_DELAYS[phase]) return phase
    remaining -= FLEX_DELAYS[phase]
  }
  return -1
}

export function flexDelay(elapsed) {
  if (!Number.isFinite(elapsed) || elapsed < 0 || elapsed >= FLEX_DURATION) return Infinity
  let remaining = elapsed
  for (const delay of FLEX_DELAYS) {
    if (remaining < delay) return delay - remaining
    remaining -= delay
  }
  return Infinity
}

export function flexMotion() {
  return NORMAL
}

// The crest art leaves 15–70px clear at the bottom of its 200px frame, so it overlaps the frame's
// bottom by FLEX_DROP to sit closer to the hero's head without covering it.
export const FLEX_DROP = 30

// The large crest is centered above the hero's head. Keep the whole effect in
// the 240px overlay window even when the hero stands at a screen edge.
export function flexPlace(spriteX, spriteY, windowWidth = 240) {
  return {
    x: Math.max(0, Math.min(windowWidth - FLEX_WIDTH, spriteX + 64 - FLEX_WIDTH / 2)),
    y: spriteY - FLEX_HEIGHT + FLEX_DROP,
  }
}

// A visiting hero can stand near the edge of the 240px window. The outer
// particles may leave it, but the badge itself must stay over the sender.
export function visitorFlexPlace(spriteX, spriteY, league = 'bronze') {
  const width = Math.round(FLEX_WIDTH * VISITOR_FLEX_SCALE)
  const height = Math.round(FLEX_HEIGHT * VISITOR_FLEX_SCALE)
  return {
    x: spriteX + 64 - width / 2,
    y: spriteY - FLEX_HEIGHT + FLEX_DROP + FLEX_HEIGHT - height - (league === 'bronze' ? 0 : 18),
    width,
    height,
  }
}

export function flexLabelMaxWidth(center, windowWidth = 240) {
  return Math.max(0, Math.min(FLEX_WIDTH - 16, 2 * Math.min(center, windowWidth - center)))
}

// The crest shows the player's leaderboard rank, or nothing while the server sends none.
export function flexLabel(rank) {
  return Number.isSafeInteger(rank) && rank > 0 ? `#${rank.toLocaleString('en-US')}` : null
}
