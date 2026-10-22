// Timing and geometry of the approved rank crest animation. Each sheet is twelve
// transparent 180x200 frames, top to bottom. The sprite and crest stay separate.
export const FLEX_DELAYS = [80, 110, 100, 120, 145, 250, 170, 340, 340, 340, 150, 60]
export const FLEX_DURATION = FLEX_DELAYS.reduce((sum, delay) => sum + delay, 0)
export const FLEX_WIDTH = 180
export const FLEX_HEIGHT = 200
const NORMAL = { dx: 0, dy: 0, width: 128, height: 128 }
const MOTION = [
  NORMAL,
  { dx: -3, dy: 6, width: 134, height: 122 }, // anticipation: feet stay planted
  { dx: 1, dy: -10, width: 126, height: 132 }, // launch
  { dx: -1, dy: -6, width: 130, height: 130 },
  { dx: 0, dy: -2, width: 128, height: 128 },
  NORMAL,
  { dx: 0, dy: 2, width: 128, height: 126 }, // landing squash
  { dx: 0, dy: -1, width: 128, height: 129 },
  NORMAL,
  { dx: 0, dy: -1, width: 128, height: 129 },
  NORMAL,
  NORMAL,
]

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
  const phase = flexPhase(elapsed)
  return phase < 0 ? NORMAL : MOTION[phase]
}

// The large crest is centered above the hero's head. Keep the whole effect in
// the 240px overlay window even when the hero stands at a screen edge.
export function flexPlace(spriteX, spriteY, windowWidth = 240) {
  return {
    x: Math.max(0, Math.min(windowWidth - FLEX_WIDTH, spriteX + 64 - FLEX_WIDTH / 2)),
    y: spriteY - FLEX_HEIGHT,
  }
}
