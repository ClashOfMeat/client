import test from 'node:test'
import assert from 'node:assert/strict'
import { XP_COLOR, xpTextFrame, drawXpText } from '../xp-text.mjs'

const area = { centerX: 100, areaHeight: 100 }

test('formats raw XP and uses the source X bitmap', () => {
  const frame = xpTextFrame(240, 0, area)
  assert.equal(frame.text, 'XP: +240')
  assert.equal(frame.width, 94)
  assert.equal(frame.height, 14)
  assert.equal(frame.scale, 2)
  const xPixels = frame.pixels.filter(([x]) => x < 5)
  assert.deepEqual(xPixels, [
    [0, 0], [4, 0], [1, 1], [3, 1], [2, 2], [2, 3], [2, 4], [1, 5], [3, 5], [0, 6], [4, 6]
  ])
})

test('rises and fades over 1800 ms', () => {
  assert.equal(xpTextFrame(240, 0, area).top, 86)
  assert.equal(xpTextFrame(240, 1799, area).top, 0)
  assert.equal(xpTextFrame(240, 1000, area).alpha, 1)
  assert.equal(xpTextFrame(240, 1625, area).alpha, 0.5)
  assert.equal(xpTextFrame(240, 1800, area), null)
  assert.equal(xpTextFrame(240, -1, area), null)
})

test('clamps left inside maxWidth', () => {
  assert.equal(xpTextFrame(240, 0, { ...area, centerX: -10, maxWidth: 120 }).left, 0)
  assert.equal(xpTextFrame(240, 0, { ...area, centerX: 200, maxWidth: 120 }).left, 26)
  assert.equal(xpTextFrame(240, 0, { ...area, centerX: 200, maxWidth: 50 }).left, 0)
})

test('draws scaled pixels with the XP color and alpha', () => {
  const calls = []
  const ctx = {
    save() { calls.push(['save']) },
    restore() { calls.push(['restore']) },
    fillRect(...rect) { calls.push(['fillRect', ...rect]) }
  }
  const frame = xpTextFrame(240, 1625, area)
  drawXpText(ctx, frame)
  assert.equal(ctx.globalAlpha, 0.5)
  assert.equal(ctx.fillStyle, XP_COLOR)
  assert.deepEqual(calls[0], ['save'])
  assert.deepEqual(calls[1], ['fillRect', frame.left, frame.top, 2, 2])
  assert.equal(calls.filter(([name]) => name === 'fillRect').length, frame.pixels.length)
  assert.deepEqual(calls.at(-1), ['restore'])
  calls.length = 0
  drawXpText(ctx, null)
  assert.deepEqual(calls, [])
})

test('rejects invalid XP amounts', () => {
  for (const amount of [0, -1, 1.5, '240', Number.MAX_SAFE_INTEGER + 1, NaN]) {
    assert.equal(xpTextFrame(amount, 0, area), null)
  }
})
