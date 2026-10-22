import test from 'node:test'
import assert from 'node:assert/strict'
import { FLEX_DELAYS, FLEX_DURATION, flexDelay, flexMotion, flexPhase, flexPlace } from '../flex.mjs'

test('rank crest plays all twelve phases and leaves after the final lift', () => {
  assert.equal(FLEX_DURATION, 2205)
  let at = 0
  for (let phase = 0; phase < FLEX_DELAYS.length; phase++) {
    assert.equal(flexPhase(at), phase)
    assert.equal(flexPhase(at + FLEX_DELAYS[phase] - 1), phase)
    assert.equal(flexDelay(at), FLEX_DELAYS[phase])
    assert.equal(flexDelay(at + FLEX_DELAYS[phase] - 1), 1)
    at += FLEX_DELAYS[phase]
  }
  assert.equal(flexPhase(at), -1)
  assert.equal(flexDelay(at), Infinity)
})

test('rank crest stays above the sprite and inside the overlay at both screen edges', () => {
  assert.deepEqual(flexPlace(0, 252), { x: 0, y: 52 })
  assert.deepEqual(flexPlace(56, 252), { x: 30, y: 52 })
  assert.deepEqual(flexPlace(112, 252), { x: 60, y: 52 })
})

test('hero anticipates, rises, lands, and settles while the crest plays', () => {
  const crouch = flexMotion(FLEX_DELAYS[0])
  assert.equal(crouch.dy + crouch.height, 128)
  assert(crouch.width > 128)
  const launch = flexMotion(FLEX_DELAYS[0] + FLEX_DELAYS[1])
  assert(launch.dy + launch.height < 128)
  const landed = flexMotion(FLEX_DELAYS.slice(0, 6).reduce((sum, ms) => sum + ms, 0))
  assert.equal(landed.dy + landed.height, 128)
  assert.deepEqual(flexMotion(FLEX_DURATION), { dx: 0, dy: 0, width: 128, height: 128 })
})
