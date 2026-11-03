import test from 'node:test'
import assert from 'node:assert/strict'
import { FLEX_DELAYS, FLEX_DURATION, flexDelay, flexLabel, flexLabelMaxWidth, flexMotion, flexPhase, flexPlace, visitorFlexPlace } from '../flex.mjs'

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

test('rank crest sits close above the sprite and inside the overlay at both screen edges', () => {
  assert.deepEqual(flexPlace(0, 252), { x: 0, y: 82 })
  assert.deepEqual(flexPlace(56, 252), { x: 30, y: 82 })
  assert.deepEqual(flexPlace(112, 252), { x: 60, y: 82 })
})

test('visiting Flex badge follows the sending hero instead of clamping between the pair', () => {
  assert.deepEqual(visitorFlexPlace(140, 252), { x: 132, y: 122, width: 144, height: 160 })
  assert.deepEqual(visitorFlexPlace(28, 252), { x: 20, y: 122, width: 144, height: 160 })
  assert.deepEqual(visitorFlexPlace(140, 252, 'goose'), { x: 132, y: 104, width: 144, height: 160 })
  assert.equal(flexPlace(140, 252).x, 60)
  assert.equal(flexLabelMaxWidth(204, 240), 72)
  assert.equal(flexLabelMaxWidth(120, 240), 164)
})

test('the crest shows the leaderboard rank, never a score', () => {
  assert.equal(flexLabel(12), '#12')
  assert.equal(flexLabel(1234), '#1,234')
  for (const rank of [0, -1, 1.5, NaN, undefined, null, '3']) assert.equal(flexLabel(rank), null)
})

test('hero keeps a fixed draw size throughout all four Flex poses', () => {
  const normal = { dx: 0, dy: 0, width: 128, height: 128 }
  for (const elapsed of [0, 299, 300, 424, 425, 549, 550, 3549, FLEX_DURATION])
    assert.deepEqual(flexMotion(elapsed), normal, `Flex draw at ${elapsed} ms`)
})
