const SPRITE_SIZE = 128

const clamp = (value, min, max) => Math.max(min, Math.min(max, value))

// Keep the large heart beside the whole sprite, including every skin's head and hands.
// 56px is the space on either side of a centered 128px sprite in the 240px overlay.
export function kissHeartPlace(spriteX, spriteY, windowWidth, facing, elapsed) {
  const rise = clamp((elapsed - 520) / 450, 0, 1)
  const sideRoom = Math.floor((windowWidth - SPRITE_SIZE) / 2)
  if (sideRoom < 1) return null
  const size = Math.min(52 + Math.round(4 * Math.sin(Math.PI * rise)), sideRoom)
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
