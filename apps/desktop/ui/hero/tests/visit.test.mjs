import test from 'node:test'
import assert from 'node:assert/strict'
import { visitTracker, visitLabelLeft, visitAnchor, anchoredRect, edgeFade } from '../visit.mjs'

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

test('a visit is pinned where the visitor arrived, even when the host walks away', () => {
  const arrival = { x: 500, y: 600, width: 240, height: 380, spriteX: 56, spriteY: 252 }
  const spot = visitAnchor(arrival)
  assert.deepEqual(spot, { x: 500, spriteX: 56, stand: 84, edge: 184 })
  // The host has walked 150 px right: its own window would follow, the pinned one does not.
  const walked = { ...arrival, x: 650 }
  const pinned = anchoredRect(walked, spot)
  assert.equal(pinned.x, 500)
  assert.equal(pinned.spriteX, 206)
  assert.equal(pinned.y, walked.y)
  // Where the host has not moved, pinning changes nothing.
  assert.deepEqual(anchoredRect(arrival, spot), arrival)
})

test('a host leaving a pinned window fades at its edge', () => {
  assert.equal(edgeFade(56, 240), 1)
  assert.equal(edgeFade(112, 240), 1)
  assert.equal(edgeFade(144, 240), 0.5)
  assert.equal(edgeFade(176, 240), 0)
  assert.equal(edgeFade(-64, 240), 0)
  assert.equal(edgeFade(400, 240), 0)
})
