// Coordinates and timestamps are supplied by the overlay; this module owns no timer.
export function heroActivity(now = Date.now(), { random = Math.random } = {}) {
  let clock = now, previous = now, paused = false
  let state = 'idle', started = now, lastActivity = now, talkStarted = null, manual
  let x = 0, facing = 'right', width = 0, working = false
  let strollAt = Infinity, target = null, movedAt = now, emoteUntil = null, postponed = false
  let sessions = new Map()

  function advance(at, freeze = paused) {
    const delta = Math.max(0, at - previous)
    previous = Math.max(previous, at)
    if (!freeze) clock += delta
  }

  function setState(next, at = clock) {
    if (state !== next) { state = next; started = at }
  }

  function schedule(at = clock) {
    postponed = false
    strollAt = working ? at + 12000 + random() * 28000 : Infinity
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

  function finishEmote() {
    if (emoteUntil === null || clock < emoteUntil) return
    if (strollAt <= emoteUntil) strollAt = emoteUntil
    lastActivity = emoteUntil
    const ended = emoteUntil
    emoteUntil = null
    if (target !== null) setState('walk', ended)
    else if (working) setState('idle', ended)
    else {
      const [next, since] = settledState()
      setState(next, since)
    }
  }

  function startStroll(at, mayTurn = true) {
    strollAt = Infinity
    if (mayTurn && random() < 0.25) {
      facing = facing === 'left' ? 'right' : 'left'
      schedule(at)
      return
    }
    const distance = 60 + random() * 240
    let direction = random() < 0.5 ? -1 : 1
    if (direction < 0 && x < distance || direction > 0 && width - x < distance) direction *= -1
    target = Math.max(0, Math.min(width, x + direction * distance))
    if (target === x) {
      target = null
      schedule(at)
      return
    }
    movedAt = at
    facing = direction > 0 ? 'right' : 'left'
    setState('walk', at)
  }

  return {
    update(activity, at) {
      advance(at)
      finishEmote()
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
      const busy = !!activity?.busy || [...current.values()].includes('Busy')
      if (busy) {
        lastActivity = clock
        talkStarted = null
        if (!working && target === null) {
          working = true
          schedule()
        }
        working = true
        if (emoteUntil === null) setState(target === null ? 'idle' : 'walk')
      } else {
        strollAt = Infinity
        if (completed) {
          lastActivity = clock
          talkStarted = clock
        } else if (working) {
          lastActivity = clock
          talkStarted = null
        }
        working = false
        if (emoteUntil === null && target === null) {
          const [next, since] = settledState()
          setState(next, since)
        }
      }
    },
    snapshot(at, { width: available = width, speed = 48, paused: freeze = false, canMirror = true, still = false } = {}) {
      advance(at, freeze)
      paused = freeze
      width = Number.isFinite(available) ? Math.max(0, available) : 0
      x = Math.min(x, width)
      if (!freeze) {
        finishEmote()
        const rate = Number.isFinite(speed) ? Math.max(0, speed) : 0
        while (true) {
          if (target !== null) {
            target = Math.min(target, width)
            const arrival = canMirror && rate > 0 ? movedAt + Math.abs(target - x) * 1000 / rate : Infinity
            if (x === target || arrival <= clock) {
              x = target
              movedAt = Math.min(clock, arrival)
              target = null
              schedule(movedAt)
              if (emoteUntil === null) {
                if (working) setState('idle', movedAt)
                else {
                  const [next, since] = settledState()
                  setState(next, since)
                }
              }
              continue
            }
            if (canMirror) x = target > x ? Math.min(target, x + (clock - movedAt) * rate / 1000) : Math.max(target, x - (clock - movedAt) * rate / 1000)
            movedAt = clock
            break
          }
          if (working && clock >= strollAt) {
            if (still || emoteUntil !== null || manual !== undefined) postponed = true
            else {
              startStroll(postponed ? clock : strollAt)
              continue
            }
          }
          break
        }
        if (emoteUntil === null && manual === undefined && target === null) {
          if (working) setState('idle')
          else {
            const [next, since] = settledState()
            setState(next, since)
          }
        } else if (manual !== undefined && emoteUntil === null) setState(manual)
      }
      let until = Infinity
      if (emoteUntil !== null) until = Math.max(0, emoteUntil - clock)
      else if (target === null && manual === undefined) {
        if (working) until = postponed ? Infinity : Math.max(0, strollAt - clock)
        else if (state === 'talk') until = Math.max(0, talkStarted + 1500 - clock)
        else if (state === 'idle') until = Math.max(0, lastActivity + 20000 - clock)
        else if (state === 'sit') until = Math.max(0, lastActivity + 90000 - clock)
      }
      return { state, elapsed: Math.max(0, clock - started), x, facing, until }
    },
    drag(position, at) {
      advance(at)
      finishEmote()
      x = Number.isFinite(position) ? Math.max(0, Math.min(width, position)) : x
      if (target !== null) {
        target = null
        schedule()
        if (emoteUntil === null) setState('idle')
      }
    },
    preview(pose, at) {
      advance(at)
      finishEmote()
      manual = pose === 'idle' ? undefined : pose
      lastActivity = clock
      talkStarted = null
      setState(manual ?? 'idle')
    },
    // Starts a real stroll at once, even between agent turns, so "Start walking" shows its effect.
    strollNow(at) {
      advance(at)
      finishEmote()
      if (target !== null || emoteUntil !== null) return
      manual = undefined
      lastActivity = clock
      startStroll(clock, false)
    },
    emote(pose, at, durationMs) {
      advance(at)
      if (target !== null) {
        target = null
        schedule()
      }
      emoteUntil = clock + durationMs
      setState(pose)
    },
  }
}
