import test from 'node:test'
import assert from 'node:assert/strict'
import { plantPose, signPostX, PLANT_MS, SIGN_DROP_AT, SIGN_LANDS_AT } from '../plant.mjs'

test('the plant crouches, hops, stomps as the sign lands, then talks', () => {
  assert.deepEqual(plantPose(0), { frame: 0, sx: 1, sy: 1, dy: 0, toHost: false })
  assert.equal(plantPose(250).dy < -8, true)
  assert.equal(plantPose(SIGN_LANDS_AT).sy, 0.88)
  assert.equal(SIGN_LANDS_AT - SIGN_DROP_AT, 300)
  assert.deepEqual(plantPose(760), { frame: 4, sx: 1, sy: 1, dy: 0, toHost: true })
  assert.equal(plantPose(910).frame, 5)
  assert.equal(plantPose(PLANT_MS + 500).toHost, true)
  assert.equal(plantPose(-100).frame, 0)
})

test('the post stands beside the visitor, away from the host, on the screen', () => {
  const area = { x: 0, y: 25, width: 1440, height: 875 }
  assert.equal(signPostX(800, 1, area), 910)
  assert.equal(signPostX(800, -1, area), 690)
  assert.equal(signPostX(1400, 1, area), 1440 - 78)
  assert.equal(signPostX(20, -1, area), 78)
})
