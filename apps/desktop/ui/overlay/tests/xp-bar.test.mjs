import test from 'node:test'
import assert from 'node:assert/strict'
import { compactAmount, xpBar } from '../xp-bar.mjs'

test('amounts compact like the web app gauge and never round up', () => {
  assert.equal(compactAmount(15_500_000), '15.5M')
  assert.equal(compactAmount(15_499_999), '15.4M')
  assert.equal(compactAmount(1_999), '1K')
  assert.equal(compactAmount(950), '950')
  assert.equal(compactAmount(0), '0')
})

test('progress shows the XP into this level over the level span', () => {
  const bar = xpBar({ level: 4, levelFloor: 1_000, nextLevelAt: 3_500 }, 1_250)
  assert.deepEqual(bar, { state: 'progress', level: 'Lv 4', count: '250/2K', fill: 10 })
  assert.equal(xpBar({ level: 1, nextLevelAt: 15_500_000 }, 0).count, '0/15.5M')
})

test('the count and fill stay inside the bar', () => {
  assert.equal(xpBar({ level: 2, levelFloor: 100, nextLevelAt: 200 }, 250).fill, 100)
  assert.equal(xpBar({ level: 2, levelFloor: 100, nextLevelAt: 200 }, 50).count, '0/100')
})

test('max level fills the bar and missing XP empties it', () => {
  assert.deepEqual(xpBar({ level: 30, nextLevelAt: null }, 2_400_000), { state: 'max', level: 'Lv 30', count: 'Max · 2.4M XP', fill: 100 })
  assert.deepEqual(xpBar({ level: 3, nextLevelAt: 100 }, undefined), { state: 'unavailable', level: 'Lv 3', count: 'XP unavailable', fill: 0 })
})
