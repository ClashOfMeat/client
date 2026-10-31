import test from 'node:test'
import assert from 'node:assert/strict'
import { beginDeparture, departurePose, DEPARTURE_OVERHANG, DEPARTURE_STEP_MS } from '../departure.mjs'

test('a local send walks to the nearest edge, then disappears', () => {
  const departure = beginDeparture({ x: 280, width: 1000, facing: 'right', startedAt: 1000 })
  assert.equal(departure.edge, 0)
  assert.equal(departure.exitX, -DEPARTURE_OVERHANG)
  const start = departurePose(departure, 1000)
  assert.deepEqual([start.state, start.x, start.facing, start.opacity, start.done, start.nextIn],
    ['walk', 280, 'left', 1, false, DEPARTURE_STEP_MS])
  const middle = departurePose(departure, 1000 + departure.durationMs / 2)
  assert.equal(middle.x, 76)
  assert.equal(middle.opacity, 1)
  const end = departurePose(departure, 1000 + departure.durationMs)
  assert.deepEqual([end.x, end.opacity, end.done, end.nextIn], [-DEPARTURE_OVERHANG, 0, true, Infinity])
})

test('ties follow the current facing and the hero fades only near the edge', () => {
  const left = beginDeparture({ x: 500, width: 1000, facing: 'left', startedAt: 0 })
  const right = beginDeparture({ x: 500, width: 1000, facing: 'right', startedAt: 0 })
  assert.deepEqual([left.edge, right.edge], [0, 1000])
  assert.deepEqual([left.exitX, right.exitX], [-128, 1128])
  assert.equal(departurePose(right, right.durationMs * 0.75).opacity, 1)
  assert.equal(departurePose(right, right.durationMs * 0.875).opacity, 0.5)
})

test('a hero already at the edge still takes visible steps before fading', () => {
  const departure = beginDeparture({ x: 0, width: 0, facing: 'left', startedAt: 0 })
  assert.equal(departure.durationMs, 650)
  assert.deepEqual([departurePose(departure, 0).x, departurePose(departure, 0).facing], [0, 'left'])
  assert(departurePose(departure, 300).x < 0)
  assert.equal(departurePose(departure, 600).done, false)
  assert.deepEqual([departurePose(departure, 650).x, departurePose(departure, 650).done], [-128, true])
})
