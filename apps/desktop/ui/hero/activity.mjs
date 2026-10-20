// Coordinates and timestamps are supplied by the overlay; this module owns no timer.
export function heroActivity(now = Date.now()) {
  let clock = now, previous = now, paused = false
  let state = 'idle', started = now, lastActivity = now, talkStarted = null, manual
  let x = 0, facing = 'right', direction = 1, width = 0
  let sessions = new Map()

  function advance(at, freeze = paused) {
    const delta = Math.max(0, at - previous)
    previous = Math.max(previous, at)
    if (!freeze) clock += delta
    return freeze ? 0 : delta
  }

  function setState(next, at = clock) {
    if (state !== next) { state = next; started = at }
  }

  function settledState() {
    let idleStarted = lastActivity
    if (talkStarted !== null) {
      if (clock < talkStarted + 1500) return ['talk', talkStarted]
      idleStarted = talkStarted + 1500
      talkStarted = null
    }
    const inactive = clock - lastActivity
    if (inactive >= 90000) return ['sleep', lastActivity + 90000]
    if (inactive >= 20000) return ['sit', lastActivity + 20000]
    return ['idle', idleStarted]
  }

  return {
    update(activity, at) {
      advance(at)
      const current = new Map()
      let completed = false
      for (const session of activity?.sessions ?? []) {
        const key = `${session.provider}\0${session.session_id}`
        current.set(key, session.state)
        if (sessions.get(key) === 'Busy' && session.state === 'Done') completed = true
      }
      sessions = current
      if (manual !== undefined) lastActivity = clock
      manual = undefined
      if (activity?.busy || [...current.values()].includes('Busy')) {
        lastActivity = clock
        talkStarted = null
        setState('walk')
      } else if (completed) {
        lastActivity = clock
        talkStarted = clock
        setState('talk')
      } else if (state === 'walk') {
        lastActivity = clock
        talkStarted = null
        setState('idle')
      }
    },
    snapshot(at, { width: available = width, speed = 48, paused: freeze = false, canMirror = true } = {}) {
      const delta = advance(at, freeze)
      paused = freeze
      width = Number.isFinite(available) ? Math.max(0, available) : 0
      x = Math.min(x, width)
      if (!freeze) {
        if (manual !== undefined) setState(manual)
        else if (state !== 'walk') {
          const [next, since] = settledState()
          setState(next, since)
        }
        if (state === 'walk' && canMirror && width > 0) {
          const distance = delta * (Number.isFinite(speed) ? Math.max(0, speed) : 0) / 1000
          let phase = direction > 0 ? x : 2 * width - x
          phase = (phase + distance) % (2 * width)
          if (phase < width) { x = phase; direction = 1 }
          else if (phase > width) { x = 2 * width - phase; direction = -1 }
          else { x = width; direction = -1 }
          facing = direction > 0 ? 'left' : 'right'
        }
      }
      return { state, elapsed: Math.max(0, clock - started), x, facing }
    },
    drag(position, at) {
      advance(at)
      x = Number.isFinite(position) ? Math.max(0, Math.min(width, position)) : x
      if (x === 0) direction = 1
      if (x === width && width > 0) direction = -1
    },
    preview(pose, at) {
      advance(at)
      manual = pose === 'idle' ? undefined : pose
      lastActivity = clock
      talkStarted = null
      setState(manual ?? 'idle')
    },
  }
}
