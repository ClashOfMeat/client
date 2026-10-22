import { companionFrame } from './frames.mjs'

// The plugin counts the positive XP difference on the same character, including
// level changes. The server does not provide the intervening level thresholds.
export function xpGain(previousCharacter, character, at) {
  if (!previousCharacter || !character || previousCharacter.id !== character.id ||
      !Number.isSafeInteger(previousCharacter.xp) || !Number.isSafeInteger(character.xp) ||
      character.xp <= previousCharacter.xp) return null
  return { amount: character.xp - previousCharacter.xp, startedAt: at }
}

export function bubbleFrame(bubble, at) {
  const elapsed = at - bubble.startedAt
  if (elapsed < 0 || elapsed >= 1800) return { visible: false, dy: -32, opacity: 0 }
  return { visible: true, dy: elapsed === 0 ? 0 : -32 * elapsed / 1800, opacity: Math.min(1, (1800 - elapsed) / 350) }
}

export function levelUpFrame(effect, elapsed) {
  let time = Math.max(0, elapsed), frame = 0
  while (frame < effect.durations.length - 1 && time >= effect.durations[frame]) time -= effect.durations[frame++]
  return frame
}

// Offsets are pixels from the host hero: the visitor walks in from `edge` to `stand` beside it,
// plays its emote (or a kiss when its animation lacks that clip) turned towards the host, and walks
// back out. It always faces the way it walks; base frames face right.
export function visitPose(animation, elapsed, stand, edge, emote = 'kiss') {
  const arriving = elapsed < 1500, leaving = elapsed >= 6500
  const state = arriving || leaving ? 'walk' : animation?.clips?.[emote] ? emote : 'kiss'
  const progress = arriving ? Math.max(0, elapsed) / 1500 : leaving ? Math.min(1, (elapsed - 6500) / 1500) : 0
  const inward = edge > stand ? 'left' : 'right', outward = inward === 'left' ? 'right' : 'left'
  const clip = animation?.clips?.[state]
  const duration = clip?.durations.reduce((sum, ms) => sum + ms, 0) ?? 0
  const clipElapsed = state === 'flex' && duration ? Math.min(elapsed - 1500, duration - 1) : arriving || leaving ? elapsed : elapsed - 1500
  return { frame: companionFrame(animation, state, clipElapsed), state,
    facing: arriving ? inward : leaving ? outward : stand > 0 ? 'left' : 'right',
    offset: arriving ? edge + (stand - edge) * progress : leaving ? stand + (edge - stand) * progress : stand }
}

// Where a visitor stands in the fixed window: beside the host on the side with room, close enough
// to overlap a little (sprites have transparent margins), entering from that side's window edge.
export function visitPlace(spriteX, windowWidth, sprite = 128, gap = 84) {
  const right = windowWidth - sprite - spriteX
  if (right >= gap - 12 || right >= spriteX) return { stand: Math.max(0, Math.min(gap, right)), edge: right + sprite }
  return { stand: -Math.min(gap, spriteX), edge: -(spriteX + sprite) }
}
