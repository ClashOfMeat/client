import test from 'node:test'
import assert from 'node:assert/strict'
import { HERO_CELL_SIZE, LEGACY_FRAME_SIZE, heroDrawRect, heroEffectAnchor, heroFrameSize } from '../display.mjs'

test('both contract versions draw at native size on the same center and foot line', () => {
  assert.equal(heroFrameSize({ version: 2 }), LEGACY_FRAME_SIZE)
  assert.equal(heroFrameSize({ version: 3 }), HERO_CELL_SIZE)
  const legacy = heroDrawRect(56, 252, LEGACY_FRAME_SIZE)
  const current = heroDrawRect(56, 252, HERO_CELL_SIZE)
  assert.deepEqual(legacy, { x: 88, y: 316, width: 128, height: 128 })
  assert.deepEqual(current, { x: 56, y: 252, width: 192, height: 192 })
  for (const rect of [legacy, current]) {
    assert.equal(rect.x + rect.width / 2, 152)
    assert.equal(rect.y + rect.height, 444)
  }
  assert.deepEqual(heroEffectAnchor(56, 252), { x: 88, y: 316 })
})
