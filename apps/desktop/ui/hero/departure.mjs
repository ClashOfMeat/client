// A successful local send gets one visible walk toward the nearest screen edge.
// The caller keeps the visit's pending/delivered state separately and hides the
// hero once this short departure finishes. Restored visits do not start one.
export const DEPARTURE_STEP_MS = 83
export const DEPARTURE_OVERHANG = 128

const clamp = (value, min, max) => Math.min(max, Math.max(min, value))

export function beginDeparture({ x, width, facing = 'right', startedAt }) {
  const travelWidth = Number.isFinite(width) ? Math.max(0, width) : 0
  const startX = clamp(Number.isFinite(x) ? x : 0, 0, travelWidth)
  const left = startX < travelWidth / 2 || (startX === travelWidth / 2 && facing === 'left')
  const edge = left ? 0 : travelWidth
  const exitX = edge + (left ? -DEPARTURE_OVERHANG : DEPARTURE_OVERHANG)
  const distance = Math.abs(exitX - startX)
  const durationMs = clamp(Math.ceil(distance / 280 * 1000), 650, 2400)
  return { startX, edge, exitX, facing: left ? 'left' : 'right', startedAt, durationMs }
}

export function departurePose(departure, now) {
  const elapsed = clamp(now - departure.startedAt, 0, departure.durationMs)
  const progress = elapsed / departure.durationMs
  return {
    state: 'walk',
    elapsed,
    x: departure.startX + (departure.exitX - departure.startX) * progress,
    facing: departure.facing,
    opacity: clamp((1 - progress) * 4, 0, 1),
    done: elapsed >= departure.durationMs,
    nextIn: elapsed >= departure.durationMs ? Infinity : Math.min(DEPARTURE_STEP_MS, departure.durationMs - elapsed),
  }
}
