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
    snapshot(now = Date.now()) {
      expire(now)
      return [
        ...[...finished.values()].filter(visit => Date.parse(visit.expiresAt) > now),
        ...[...active].map(([id, expiresAt]) => ({ id, status: 'pending', expiresAt: new Date(expiresAt).toISOString() })),
      ]
    },
  }
}
