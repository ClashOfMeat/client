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
  assert.equal(visitLabelLeft(116, 60, 424), 182)
  assert.equal(visitLabelLeft(-40, 60, 424), 26)
  assert.equal(visitLabelLeft(300, 60, 424), 362)
  assert.equal(visitLabelLeft(350, 60, 424), null)
  assert.equal(visitLabelLeft(-100, 60, 424), null)
})

test('a visit is pinned where the visitor arrived, even when the host walks away', () => {
  const arrival = { x: 500, y: 600, width: 424, height: 444, spriteX: 116, spriteY: 252 }
  const spot = visitAnchor(arrival)
  assert.deepEqual(spot, { x: 500, spriteX: 116, stand: 112, edge: 308 })
  // The host has walked 150 px right: its own window would follow, the pinned one does not.
  const walked = { ...arrival, x: 650 }
  const pinned = anchoredRect(walked, spot)
  assert.equal(pinned.x, 500)
  assert.equal(pinned.spriteX, 266)
  assert.equal(pinned.y, walked.y)
  // Where the host has not moved, pinning changes nothing.
  assert.deepEqual(anchoredRect(arrival, spot), arrival)
})

test('a host leaving a pinned window fades at its edge', () => {
  assert.equal(edgeFade(116, 424), 1)
  assert.equal(edgeFade(232, 424), 1)
  assert.equal(edgeFade(280, 424), 0.5)
  assert.equal(edgeFade(328, 424), 0)
  assert.equal(edgeFade(-96, 424), 0)
  assert.equal(edgeFade(500, 424), 0)
})
