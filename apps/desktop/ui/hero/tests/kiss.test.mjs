import test from 'node:test'
import assert from 'node:assert/strict'
import { kissHeartPlace, visitorKissHeartPlace } from '../kiss.mjs'
import { visitPlace } from '../effects.mjs'

const overlaps = (a, b) => a.x < b.x + b.width && a.x + a.size > b.x && a.y < b.y + b.height && a.y + a.size > b.y

test('large Kiss heart stays outside the sprite at both edges and in the center', () => {
  for (const spriteX of [0, 28, 56, 84, 112]) {
    for (const facing of ['left', 'right']) {
      for (const elapsed of [520, 632, 745, 857, 970]) {
        const heart = kissHeartPlace(spriteX, 252, 240, facing, elapsed)
        assert(heart.x >= 0 && heart.x + heart.size <= 240, `${spriteX}/${facing}/${elapsed}: horizontal clipping`)
        assert(heart.y >= 0 && heart.y + heart.size <= 380, `${spriteX}/${facing}/${elapsed}: vertical clipping`)
        assert(!overlaps(heart, { x: spriteX, y: 252, width: 128, height: 128 }), `${spriteX}/${facing}/${elapsed}: sprite overlap`)
        assert(heart.size >= 39 && heart.size <= 42)
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

test('Kiss heart pulses at 75% of its previous size', () => {
  assert.deepEqual([520, 632, 745, 857, 970].map(elapsed => kissHeartPlace(56, 252, 240, 'right', elapsed).size), [39, 41, 42, 41, 39])
})

test('a visitor\'s large Kiss heart sits on its far side, clear of the host and its name label', () => {
  for (const hostX of [0, 20, 56, 84, 112]) {
    const { stand } = visitPlace(hostX, 240)
    const visitorX = hostX + stand
    const facing = stand > 0 ? 'left' : 'right'
    for (const elapsed of [520, 745, 970]) {
      const heart = visitorKissHeartPlace(visitorX, 252, 240, facing, elapsed)
      const at = `${hostX}/${elapsed}`
      assert(heart.x >= 0 && heart.x + heart.size <= 240 && heart.y >= 0, `${at}: clipped`)
      assert(!overlaps(heart, { x: hostX, y: 252, width: 128, height: 128 }), `${at}: covers the host`)
      assert(heart.y + heart.size <= 252 - 4, `${at}: covers the "from" label`)
      const away = heart.x + heart.size / 2 - (visitorX + 64)
      assert(stand > 0 ? away > 0 : away < 0, `${at}: on the host's side`)
      assert(heart.size >= 39 && heart.size <= 42)
    }
  }
})

test('a visitor\'s large Kiss heart goes beside it when the window has room, and rises', () => {
  assert.deepEqual(visitorKissHeartPlace(112, 252, 400, 'left', 520), { x: 240, y: 260, size: 39 })
  assert.deepEqual(visitorKissHeartPlace(112, 252, 240, 'left', 520), { x: 201, y: 207, size: 39 })
  assert.deepEqual(visitorKissHeartPlace(28, 252, 240, 'right', 520), { x: 28, y: 207, size: 39 })
  assert(visitorKissHeartPlace(112, 252, 240, 'left', 970).y < 207)
  assert.equal(visitorKissHeartPlace(112, 20, 240, 'left', 970).y, 0)
})
