// The hero has four poses: start for 300ms, two moving poses for 125ms each,
// then a still final pose for 3000ms. The twelve-frame crest spans that clip.
export const FLEX_DELAYS = [300, 250, 100, 100, 100, 100, 100, 100, 100, 2150, 100, 50]
export const FLEX_DURATION = FLEX_DELAYS.reduce((sum, delay) => sum + delay, 0)
export const FLEX_WIDTH = 180
export const FLEX_HEIGHT = 200
const NORMAL = { dx: 0, dy: 0, width: 128, height: 128 }
const MOVING = { dx: -2, dy: 2, width: 132, height: 126 }

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

export function flexMotion(elapsed) {
  return elapsed >= FLEX_DELAYS[0] && elapsed < FLEX_DELAYS[0] + FLEX_DELAYS[1] ? MOVING : NORMAL
}

// The large crest is centered above the hero's head. Keep the whole effect in
// the 240px overlay window even when the hero stands at a screen edge.
export function flexPlace(spriteX, spriteY, windowWidth = 240) {
  return {
    x: Math.max(0, Math.min(windowWidth - FLEX_WIDTH, spriteX + 64 - FLEX_WIDTH / 2)),
    y: spriteY - FLEX_HEIGHT,
  }
}
