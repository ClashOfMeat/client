import test from 'node:test'
import assert from 'node:assert/strict'
import { visitTracker, visitLabelLeft } from '../visit.mjs'

test('a re-sent visit plays once, and old IDs are forgotten past the limit', () => {
  const visits = visitTracker(2)
  assert.equal(visits.first('a'), true)
  assert.equal(visits.first('a'), false)
  assert.equal(visits.first('b'), true)
  assert.equal(visits.first('c'), true)
  assert.equal(visits.first('b'), false)
  assert.equal(visits.first('a'), true)
})

test('the visit label stays centred over the visitor and inside the window', () => {
  assert.equal(visitLabelLeft(56, 60, 240), 90)
  assert.equal(visitLabelLeft(-40, 60, 240), 2)
  assert.equal(visitLabelLeft(150, 60, 240), 178)
  assert.equal(visitLabelLeft(200, 60, 240), null)
  assert.equal(visitLabelLeft(-100, 60, 240), null)
})
