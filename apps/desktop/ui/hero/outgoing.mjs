const UUID = /^[a-f\d]{8}-[a-f\d]{4}-[a-f\d]{4}-[a-f\d]{4}-[a-f\d]{12}$/i
const ACTIVE = new Set(['pending', 'delivered'])
const TERMINAL = new Set(['played', 'expired', 'failed'])

export function parseOutgoingVisit(value, now = Date.now()) {
  if (!value || !UUID.test(value.id) || typeof value.expiresAt !== 'string') return null
  const expiresAt = Date.parse(value.expiresAt)
  if (!Number.isFinite(expiresAt) || expiresAt > now + 10 * 60_000) return null
  const status = value.status ?? 'pending' // Older successful send responses lacked status.
  if (!ACTIVE.has(status) && !TERMINAL.has(status)) return null
  return { id: value.id, expiresAt, status }
}

// Tracks the visit IDs that currently have the local hero away. A terminal ID is remembered
// briefly so an older heartbeat or a late send response cannot make the hero disappear again.
export function outgoingVisits(limit = 64) {
  const active = new Map()
  const finished = new Map()

  function finish(id, expiresAt, status = 'expired') {
    active.delete(id)
    finished.set(id, { id, status, expiresAt: new Date(expiresAt).toISOString() })
    if (finished.size > limit) finished.delete(finished.keys().next().value)
  }

  function expire(now = Date.now()) {
    let changed = false
    for (const [id, expiresAt] of active) {
      if (expiresAt <= now) {
        finish(id, expiresAt)
        changed = true
      }
    }
    return changed
  }

  return {
    observe(value, now = Date.now()) {
      expire(now)
      const visit = parseOutgoingVisit(value, now)
      if (!visit) return false
      const { id, expiresAt, status } = visit
      if (TERMINAL.has(status) || expiresAt <= now) {
        const changed = active.has(id)
        finish(id, expiresAt, status)
        return changed
      }
      if (finished.has(id)) return false
      const changed = !active.has(id)
      active.set(id, expiresAt)
      return changed
    },
    expire,
    away(now = Date.now()) { expire(now); return active.size > 0 },
    ids(now = Date.now()) { expire(now); return [...active.keys()] },
    clearActive(except = new Set()) {
      let changed = false
      for (const [id, expiresAt] of active) {
        if (except.has(id)) continue
        finish(id, expiresAt)
        changed = true
      }
      return changed
    },
    snapshot(now = Date.now()) {
      expire(now)
      return [
        ...[...finished.values()].filter(visit => Date.parse(visit.expiresAt) > now),
        ...[...active].map(([id, expiresAt]) => ({ id, status: 'pending', expiresAt: new Date(expiresAt).toISOString() })),
      ]
    },
  }
}

const HANDLE = /^[a-z0-9_]{3,24}$/

// The server's player handle identifies the paired account. Keep each account's short-lived
// visit records separate so switching accounts cannot carry an away hero to another player.
export function scopedOutgoingVisits(storage) {
  let owner = null
  let visits = outgoingVisits()
  const sentHere = new Set()
  const key = () => `clashofmeat.outgoingVisits.v2.${owner}`

  function save(now = Date.now()) {
    if (!owner) return
    try {
      const records = visits.snapshot(now)
      if (records.length) storage.setItem(key(), JSON.stringify(records))
      else storage.removeItem(key())
    } catch { /* A heartbeat can restore the state when WebView storage is unavailable. */ }
  }

  function trimSent(now) {
    const active = new Set(visits.ids(now))
    for (const id of sentHere) if (!active.has(id)) sentHere.delete(id)
  }

  return {
    owner() { return owner },
    identify(handle, now = Date.now()) {
      const next = typeof handle === 'string' && HANDLE.test(handle.toLowerCase()) ? handle.toLowerCase() : null
      if (next === owner) return false
      const wasAway = visits.away(now)
      owner = next
      visits = outgoingVisits()
      sentHere.clear()
      if (owner) {
        try {
          const saved = JSON.parse(storage.getItem(key()) ?? '[]')
          if (Array.isArray(saved)) for (const visit of saved) visits.observe(visit, now)
        } catch { /* Ignore corrupt or unavailable storage. */ }
      }
      save(now)
      return wasAway !== visits.away(now)
    },
    sent(value, now = Date.now()) {
      const changed = visits.observe(value, now)
      const parsed = parseOutgoingVisit(value, now)
      if (parsed && ACTIVE.has(parsed.status) && visits.ids(now).includes(parsed.id)) sentHere.add(parsed.id)
      save(now)
      return changed
    },
    heartbeat(value, now = Date.now(), authoritative = false) {
      if (value === null) {
        // A heartbeat fetched before a local send can arrive after its successful response.
        if (!authoritative) return false
        const changed = visits.clearActive(sentHere)
        save(now)
        return changed
      }
      const changed = visits.observe(value, now)
      trimSent(now)
      save(now)
      return changed
    },
    status(value, now = Date.now()) {
      const changed = visits.observe(value, now)
      trimSent(now)
      save(now)
      return changed
    },
    expire(now = Date.now()) {
      const changed = visits.expire(now)
      if (changed) { trimSent(now); save(now) }
      return changed
    },
    away(now = Date.now()) { return visits.away(now) },
    ids(now = Date.now()) { return visits.ids(now) },
  }
}
