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

// target and limit are pixel offsets measured from the visitor's entry side.
export function visitPose(animation, elapsed, target, limit) {
  const arriving = elapsed < 1500, leaving = elapsed >= 6500
  const state = arriving || leaving ? 'walk' : 'kiss'
  const far = Math.max(target, Math.min(limit, target + 192))
  const progress = arriving ? Math.max(0, elapsed) / 1500 : leaving ? Math.min(1, (elapsed - 6500) / 1500) : 0
  return { frame: companionFrame(animation, state, state === 'kiss' ? elapsed - 1500 : elapsed), state,
    facing: leaving ? 'left' : 'right',
    offset: arriving ? far + (target - far) * progress : leaving ? target + (far - target) * progress : target }
}
