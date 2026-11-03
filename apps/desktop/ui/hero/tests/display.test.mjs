import test from 'node:test'
import assert from 'node:assert/strict'
import { heroDrawRect, safeDisplayScale } from '../display.mjs'

test('display scale keeps hero feet and cell center fixed', () => {
  for (const scale of [0.9, 1, 1.125, 1.31]) {
    const rect = heroDrawRect(56, 252, scale)
    assert.equal(rect.y + rect.height, 380)
    assert(Math.abs(rect.x + rect.width / 2 - 120) <= 0.5)
    assert.equal(rect.width, Math.round(128 * scale))
    assert.equal(rect.height, Math.round(128 * scale))
  }
})

test('old and invalid display scales keep the 128px cell', () => {
  for (const value of [undefined, null, NaN, Infinity, 0, 10]) {
    assert.equal(safeDisplayScale(value), 1)
    assert.deepEqual(heroDrawRect(56, 252, value), { x: 56, y: 252, width: 128, height: 128 })
  }
})
