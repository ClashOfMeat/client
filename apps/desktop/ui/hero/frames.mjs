function clipFor(animation, state) { return animation?.clips?.[state] ?? animation?.clips?.idle }

// Grab has a one-time reaction before its held poses repeat. Replaying the
// neutral and pickup frames while dragging makes props reappear in midair.
function clipTime(animation, state, clip, elapsed) {
  const duration = clip.durations.reduce((a, b) => a + b, 0)
  const time = Math.max(0, elapsed)
  if (state === 'sleep' || state === 'kiss') return { time: Math.min(time, duration - 1), repeatStart: clip.frames.length }
  const repeatStart = state === 'grab' && animation?.clips?.grab === clip && clip.frames.length > 2 ? 2 : 0
  const onset = clip.durations.slice(0, repeatStart).reduce((a, b) => a + b, 0)
  return { time: time < onset ? time : onset + (time - onset) % (duration - onset), repeatStart }
}

export function companionFrame(animation, state, elapsed) {
  const clip = clipFor(animation, state)
  if (!clip) return 0
  let { time } = clipTime(animation, state, clip, elapsed)
  for (let i = 0; i < clip.frames.length; i++) {
    if (time < clip.durations[i]) return clip.frames[i]
    time -= clip.durations[i]
  }
  return clip.frames[0]
}

export function frameDelay(animation, state, elapsed) {
  const clip = clipFor(animation, state)
  if (!clip) return Infinity
  const held = state === 'sleep' || state === 'kiss'
  if (held && elapsed >= clip.durations.reduce((a, b) => a + b, 0)) return Infinity
  const { time, repeatStart } = clipTime(animation, state, clip, elapsed)
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
    const wrapped = next < clip.frames.length ? next : repeatStart + (next - clip.frames.length) % (clip.frames.length - repeatStart)
    if (clip.frames[wrapped] !== current) return delay
    delay += clip.durations[wrapped]
  }
  return Infinity
}

export function frameImage(animation, index, facing) {
  const mirrored = facing === 'left' ? animation?.mirroredFrames?.[index] : undefined
  return { png: mirrored ?? animation?.frames?.[index], flip: facing === 'left' && !mirrored }
}
