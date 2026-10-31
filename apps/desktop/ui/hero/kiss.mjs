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

// A visitor faces the host, so its large heart goes on the far side, where it covers neither hero.
// The fixed 240px window rarely leaves room beside the visitor there, so the heart then floats above
// that side of its head, clear of the "from <name>" label (top at spriteY - 4).
export function visitorKissHeartPlace(spriteX, spriteY, windowWidth, facing, elapsed) {
  const rise = clamp((elapsed - 520) / 450, 0, 1)
  const size = Math.min(heartSize(rise), windowWidth)
  if (size < 1) return null
  const beside = facing === 'left' ? spriteX + SPRITE_SIZE : spriteX - size
  if (beside >= 0 && beside + size <= windowWidth) return { x: beside, y: spriteY + 8 - rise * 12, size }
  const above = facing === 'left' ? spriteX + SPRITE_SIZE - size : spriteX
  return { x: clamp(above, 0, windowWidth - size), y: Math.max(0, spriteY - 6 - size - rise * 12), size }
}
