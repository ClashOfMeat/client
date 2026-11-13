import assert from 'node:assert/strict'
import test from 'node:test'
import { xpBar } from '../xp-bar.mjs'

test('the bar shows total XP over the next level\'s total, like the web hero page', () => {
  // On the way to level 30: 3,763 opens level 29 and 3,959 opens level 30.
  const bar = xpBar({ level: 29, levelFloor: 3_763, nextLevelAt: 3_959 }, 3_949)
  assert.equal(bar.state, 'progress')
  assert.equal(bar.level, 'Lv 29')
  assert.equal(bar.count, '3,949 / 3,959 XP')
  assert.equal(bar.fill, 3_949 / 3_959 * 100)
  assert.equal(xpBar({ level: 1, nextLevelAt: 30 }, 0).count, '0 / 30 XP')
})

test('XP past the next level stops at it, and the bar is full', () => {
  const bar = xpBar({ level: 2, levelFloor: 100, nextLevelAt: 200 }, 250)
  assert.equal(bar.count, '200 / 200 XP')
  assert.equal(bar.fill, 100)
})

test('the final level and a missing XP read as such', () => {
  // The server sends the floor as nextLevelAt at the final level; older ones sent null.
  assert.deepEqual(xpBar({ level: 30, levelFloor: 3_959, nextLevelAt: 3_959 }, 3_959), { state: 'max', level: 'Lv 30', count: '3,959 XP · MAX', fill: 100 })
  assert.equal(xpBar({ level: 30, nextLevelAt: null }, 3_959).state, 'max')
  assert.deepEqual(xpBar({ level: 3, nextLevelAt: 100 }, undefined), { state: 'unavailable', level: 'Lv 3', count: 'XP unavailable', fill: 0 })
})
