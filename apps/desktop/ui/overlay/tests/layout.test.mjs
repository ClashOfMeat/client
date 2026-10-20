import test from 'node:test'
import assert from 'node:assert/strict'
import { heroBox, windowRect, formatTokens } from '../layout.mjs'

const workArea = { x: 100, y: 50, width: 1000, height: 700 }

test('sprite feet touch the work area bottom throughout patrol', () => {
  assert.equal(heroBox({ workArea }).patrolWidth, 872)
  for (const x of [0, 50, 872, 999]) {
    const rect = windowRect(x, { workArea })
    assert.equal(rect.y + rect.spriteY + 128, 750)
    assert(rect.x >= 100 && rect.x + rect.width <= 1100)
    assert.equal(rect.x + rect.spriteX, 100 + Math.min(x, 872))
  }
})

test('bubble grows upward and preserves sprite position, including edges', () => {
  for (const x of [0, 420, 872]) {
    const compact = windowRect(x, { workArea })
    const expanded = windowRect(x, { workArea, bubble: true })
    assert.equal(expanded.x + expanded.spriteX, compact.x + compact.spriteX)
    assert.equal(expanded.y + expanded.spriteY, compact.y + compact.spriteY)
    assert.equal(expanded.width, 240)
    assert.equal(expanded.height, 340)
    assert(expanded.x >= 100 && expanded.x + expanded.width <= 1100)
  }
})

test('small displays keep the window inside their work area', () => {
  const tiny = { x: -50, y: 10, width: 100, height: 100 }
  assert.deepEqual(heroBox({ workArea: tiny, bubble: true }), {
    patrolWidth: 0, x: -50, y: 10, width: 100, height: 100, spriteX: 0, spriteY: -28,
  })
})

test('token counts use compact units', () => {
  assert.equal(formatTokens(1_400_000), '1.4M')
  assert.equal(formatTokens(950_000), '950k')
  assert.equal(formatTokens(12), '12')
  assert.equal(formatTokens(null), '')
  assert.equal(formatTokens(-1), '')
})
