const SPRITE_SIZE = 128
const HEART_SCALE = 0.75

const clamp = (value, min, max) => Math.max(min, Math.min(max, value))
const heartSize = rise => Math.round((52 + Math.round(4 * Math.sin(Math.PI * rise))) * HEART_SCALE)

// Keep the large heart beside the whole sprite, including every skin's head and hands.
// A centered 128px sprite has 56px of room on either side of the 240px overlay.
export function kissHeartPlace(spriteX, spriteY, windowWidth, facing, elapsed) {
  const rise = clamp((elapsed - 520) / 450, 0, 1)
  const sideRoom = Math.floor((windowWidth - SPRITE_SIZE) / 2)
  if (sideRoom < 1) return null
  const size = Math.min(heartSize(rise), sideRoom)
  const left = spriteX - size
  const right = spriteX + SPRITE_SIZE
  const leftFits = left >= 0
  const rightFits = right + size <= windowWidth
  const x = facing === 'left'
    ? (leftFits ? left : rightFits ? right : null)
    : (rightFits ? right : leftFits ? left : null)
  if (x === null) return null
  return { x, y: spriteY + 8 - rise * 12, size }
}

// Center the visiting heart between the two heroes, independently of which side
// the visitor enters from. Their 128px cells overlap in the fixed 240px window,
// but their visible bodies leave room between the faces for the 75% heart.
export function visitorKissHeartPlace(hostX, visitorX, spriteY, windowWidth, elapsed) {
  const rise = clamp((elapsed - 520) / 450, 0, 1)
  const size = Math.min(heartSize(rise), windowWidth)
  if (size < 1) return null
  const midpoint = (hostX + visitorX) / 2 + SPRITE_SIZE / 2
  return {
    x: clamp(Math.round(midpoint - size / 2), 0, windowWidth - size),
    y: Math.max(0, spriteY + 28 - rise * 12),
    size,
  }
}
