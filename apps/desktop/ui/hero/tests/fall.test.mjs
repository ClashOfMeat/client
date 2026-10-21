import test from 'node:test'
import assert from 'node:assert/strict'
import { fallPose, GRAVITY } from '../fall.mjs'

test('a dropped hero falls faster and faster, bounces once, then lands', () => {
  const from = 800, drop = Math.sqrt(2 * from / GRAVITY) * 1000
  const early = fallPose(from, drop / 4), late = fallPose(from, drop / 2)
  assert(early.lift < from && late.lift < early.lift)
  assert(from - early.lift < early.lift - late.lift, 'accelerates')
  const bounce = fallPose(from, drop + 60)
  assert(bounce.lift > 0 && !bounce.landed)
  assert.deepEqual(fallPose(from, drop + 1000), { lift: 0, landed: true })
})

test('a hero barely lifted or already on the ground lands at once', () => {
  assert.deepEqual(fallPose(0, 0), { lift: 0, landed: true })
  assert.deepEqual(fallPose(1, 10), { lift: 0, landed: true })
})

test('the bounce stays small even from very high', () => {
  let highest = 0
  const from = 5000, drop = Math.sqrt(2 * from / GRAVITY) * 1000
  for (let ms = drop; ms < drop + 400; ms += 5) highest = Math.max(highest, fallPose(from, ms).lift)
  assert(highest < 40)
})
