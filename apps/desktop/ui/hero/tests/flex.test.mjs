import test from 'node:test'
import assert from 'node:assert/strict'
import { FLEX_DELAYS, FLEX_DURATION, flexDelay, flexMotion, flexPhase, flexPlace } from '../flex.mjs'

test('rank crest plays all twelve phases and leaves after the final lift', () => {
  assert.equal(FLEX_DURATION, 3550)
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

test('hero moves during the middle pose and holds the final pose still for three seconds', () => {
  const normal = { dx: 0, dy: 0, width: 128, height: 128 }
  assert.deepEqual(flexMotion(299), normal)
  const moving = flexMotion(300)
  assert.equal(moving.dy + moving.height, 128)
  assert(moving.width > 128)
  assert.deepEqual(flexMotion(550), normal)
  assert.deepEqual(flexMotion(3549), normal)
  assert.deepEqual(flexMotion(FLEX_DURATION), { dx: 0, dy: 0, width: 128, height: 128 })
})
