// All heroes share one stage cell. Legacy 128px frames sit inside it at their
// native size; version 3 frames fill it at their native size. No draw scaling.
export const HERO_CELL_SIZE = 192
export const LEGACY_FRAME_SIZE = 128

export function heroFrameSize(animation) {
  return animation?.version === 3 ? HERO_CELL_SIZE : LEGACY_FRAME_SIZE
}

export function heroDrawRect(x, y, frameSize) {
  const size = frameSize === HERO_CELL_SIZE ? HERO_CELL_SIZE : LEGACY_FRAME_SIZE
  return {
    x: x + (HERO_CELL_SIZE - size) / 2,
    y: y + HERO_CELL_SIZE - size,
    width: size,
    height: size,
  }
}

// Effects still use the bottom-centered 128px part of the stage, where the
// visible heads and hands live in both authored frame formats.
export function heroEffectAnchor(x, y) {
  return { x: x + (HERO_CELL_SIZE - LEGACY_FRAME_SIZE) / 2, y: y + HERO_CELL_SIZE - LEGACY_FRAME_SIZE }
}
