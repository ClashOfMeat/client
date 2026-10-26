function clipFor(animation, state) { return animation?.clips?.[state] ?? animation?.clips?.idle }

export function companionFrame(animation, state, elapsed) {
  const clip = clipFor(animation, state)
  if (!clip) return 0
  const duration = clip.durations.reduce((a, b) => a + b, 0)
  let time = state === 'sleep' || state === 'kiss'
    ? Math.min(Math.max(0, elapsed), duration - 1)
    : Math.max(0, elapsed) % duration
  for (let i = 0; i < clip.frames.length; i++) {
    if (time < clip.durations[i]) return clip.frames[i]
    time -= clip.durations[i]
  }
  return clip.frames[0]
}

export function frameDelay(animation, state, elapsed) {
  const clip = clipFor(animation, state)
  if (!clip) return Infinity
  const duration = clip.durations.reduce((a, b) => a + b, 0)
  const held = state === 'sleep' || state === 'kiss'
  const time = held ? Math.max(0, elapsed) : Math.max(0, elapsed) % duration
  let remaining = time
  let index = clip.durations.length - 1
  for (let i = 0; i < clip.durations.length; i++) {
    if (remaining < clip.durations[i]) { index = i; break }
    remaining -= clip.durations[i]
  }
  const current = clip.frames[index]
  let delay = clip.durations[index] - remaining
  for (let step = 1; step <= clip.frames.length; step++) {
    const next = index + step
    if (held && next >= clip.frames.length) return Infinity
    const wrapped = next % clip.frames.length
    if (clip.frames[wrapped] !== current) return delay
    delay += clip.durations[wrapped]
  }
  return Infinity
}

export function frameImage(animation, index, facing) {
  const mirrored = facing === 'left' ? animation?.mirroredFrames?.[index] : undefined
  return { png: mirrored ?? animation?.frames?.[index], flip: facing === 'left' && !mirrored }
}
