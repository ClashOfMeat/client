import test from 'node:test'
import assert from 'node:assert/strict'
import { stripRect, signBox, signRegion, signTilt, slept, plantRequest, sameArea, STRIP_HEIGHT } from '../board.mjs'

const area = { x: 0, y: 25, width: 1440, height: 875 }

test('the strip lies along the bottom of the work area and the post stands on it', () => {
  assert.deepEqual(stripRect(area), { x: 0, y: 900 - STRIP_HEIGHT, width: 1440, height: STRIP_HEIGHT })
  assert.deepEqual(signBox(910, area), { left: 832, top: STRIP_HEIGHT - 131 })
  assert.deepEqual(signBox(910, { ...area, x: 1440 }), { left: -608, top: STRIP_HEIGHT - 131 })
  assert.deepEqual(signRegion({ left: 832, top: 109 }), [854, 109, 112, 137])
})

test('signs lean 2 to 5 degrees either way', () => {
  assert.equal(signTilt(() => 0), -2)
  assert.equal(signTilt(() => 0.999).toFixed(2), '5.00')
  for (let i = 0; i < 50; i++) { const tilt = Math.abs(signTilt()); assert(tilt >= 2 && tilt <= 5) }
})

test('a long gap between ticks means the computer slept', () => {
  assert.equal(slept(0, 5_000), false)
  assert.equal(slept(0, 61_000), true)
})

test('a plant request needs an ID, a review link, a sender, a place and a work area', () => {
  const good = { id: 'v1', url: 'https://github.com/a/b/pull/1', sender: 'Gab\n', x: 910, area }
  assert.deepEqual(plantRequest(good), { ...good, sender: 'Gab' })
  assert.equal(plantRequest({ ...good, sender: '' }).sender, 'A friend')
  for (const bad of [{ id: '' }, { url: 'https://evil.example/x' }, { x: NaN }, { area: { ...area, width: 0 } }, { area: null }, { sender: 3 }])
    assert.equal(plantRequest({ ...good, ...bad }), null)
  assert.equal(plantRequest(null), null)
  assert.equal(sameArea(area, { ...area }), true)
  assert.equal(sameArea(area, { ...area, x: 1 }), false)
})
