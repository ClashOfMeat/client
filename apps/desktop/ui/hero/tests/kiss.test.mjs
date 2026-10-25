import test from 'node:test'
import assert from 'node:assert/strict'
import { kissHeartPlace } from '../kiss.mjs'

const overlaps = (a, b) => a.x < b.x + b.width && a.x + a.size > b.x && a.y < b.y + b.height && a.y + a.size > b.y

test('large Kiss heart stays outside the sprite at both edges and in the center', () => {
  for (const spriteX of [0, 28, 56, 84, 112]) {
    for (const facing of ['left', 'right']) {
      for (const elapsed of [520, 632, 745, 857, 970]) {
        const heart = kissHeartPlace(spriteX, 252, 240, facing, elapsed)
        assert(heart.x >= 0 && heart.x + heart.size <= 240, `${spriteX}/${facing}/${elapsed}: horizontal clipping`)
        assert(heart.y >= 0 && heart.y + heart.size <= 380, `${spriteX}/${facing}/${elapsed}: vertical clipping`)
        assert(!overlaps(heart, { x: spriteX, y: 252, width: 128, height: 128 }), `${spriteX}/${facing}/${elapsed}: sprite overlap`)
        assert(heart.size >= 52 && heart.size <= 56)
      }
    }
  }
})

test('large Kiss heart uses the available side and rises during the pose', () => {
  const edgeLeft = kissHeartPlace(0, 252, 240, 'left', 520)
  const edgeRight = kissHeartPlace(112, 252, 240, 'right', 520)
  const centered = kissHeartPlace(56, 252, 240, 'right', 520)
  assert.equal(edgeLeft.x, 128)
  assert.equal(edgeRight.x + edgeRight.size, 112)
  assert.equal(centered.x, 184)
  assert.equal(centered.y, 260)
  assert(kissHeartPlace(0, 252, 240, 'right', 970).y < edgeLeft.y)
})

test('large Kiss heart scales down on narrow overlays and skips when there is no side room', () => {
  assert.equal(kissHeartPlace(36, 100, 200, 'right', 745).size, 36)
  assert.equal(kissHeartPlace(0, 100, 128, 'right', 745), null)
})
