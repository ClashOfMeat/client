import { HERO_CELL_SIZE } from './display.mjs'

// A visit's own emote clip (validateEmoteClip): the web stage's Kiss, Angry, Flex or cast for the sender's skin,
// played once in place of the contract's clip.

export const clipDuration = clip => clip.durations.reduce((sum, ms) => sum + ms, 0)

// The step playing `elapsed` ms into the clip; the last one holds.
export function clipStep(clip, elapsed) {
  let time = Math.max(0, elapsed)
  for (let step = 0; step < clip.durations.length; step++) {
    if (time < clip.durations[step]) return step
    time -= clip.durations[step]
  }
  return clip.durations.length - 1
}

// Milliseconds until the next step, or Infinity once the clip is over.
export function clipDelay(clip, elapsed) {
  let time = Math.max(0, elapsed)
  for (const ms of clip.durations) {
    if (time < ms) return ms - time
    time -= ms
  }
  return Infinity
}

// What the player draws over this step: the league crest for a Flex, the anger mark from `effectFrom` on, or nothing.
export function clipEffect(clip, step) {
  if (clip.effect === 'crest') return 'flex'
  if (clip.effect === 'mark' && step >= clip.effectFrom) return 'angry'
  return null
}

// Where a step's sheet frame is drawn for a hero cell at (cellX, cellY), or null for the hero's own idle (contract
// frame 0). Frames face right; facing left mirrors the frame about the cell's centre, so the hero stays in its cell.
export function clipDraw(clip, step, cellX, cellY, facing) {
  const frame = clip.frames[step]
  if (frame === null || frame === undefined) return null
  const left = facing === 'left' ? HERO_CELL_SIZE - clip.origin[0] - clip.width : clip.origin[0]
  return {
    sx: (frame % clip.columns) * clip.width, sy: Math.floor(frame / clip.columns) * clip.height,
    width: clip.width, height: clip.height,
    x: cellX + left, y: cellY + clip.origin[1], flip: facing === 'left',
  }
}

// How far the clip's frames reach past either side of the hero cell, facing either way: the hero window grows by
// this much on each side while the visitor plays it, so a wide cast (Spy Girl's L5) is never cut at the window edge.
export function clipReach(clip) {
  return Math.max(0, -clip.origin[0], clip.origin[0] + clip.width - HERO_CELL_SIZE)
}
