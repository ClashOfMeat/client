import test from 'node:test'
import assert from 'node:assert/strict'
import { clipDuration, clipStep, clipDelay, clipEffect, clipDraw, clipReach } from '../clip.mjs'
import { validateEmoteClip, validateVisit } from '../validate.mjs'
import { visitDuration, visitPose } from '../effects.mjs'
import { windowRect } from '../../overlay/layout.mjs'

const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACXBIWXMAAAPoAAAD6AG1e1JrAAAADUlEQVQImWP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg=='
const sheet = (width, height) => {
  const bytes = Buffer.from(png, 'base64')
  bytes.writeUInt32BE(width, 16)
  bytes.writeUInt32BE(height, 20)
  return bytes.toString('base64')
}
// The 3-pose Angry as the app sends it: the idle, then three 128 px poses at the hero's place in its cell.
const angry = { v: 1, revision: 'a'.repeat(32), sheet: sheet(384, 128), columns: 3, rows: 1, width: 128, height: 128,
  origin: [32, 64], frames: [null, 0, 1, 2], durations: [300, 200, 300, 1200], effect: 'mark', effectFrom: 2 }
// Spy Girl's L5 cast: 256 px frames reaching 32 px out of each side of the cell.
const cast = { v: 1, revision: 'b'.repeat(32), sheet: sheet(512, 256), columns: 2, rows: 1, width: 256, height: 256,
  origin: [-32, 0], frames: [0, 1], durations: [83, 83], effect: 'crest', effectFrom: 0 }

test('a visit clip plays once at its own timings and holds its last step', () => {
  assert.equal(clipDuration(angry), 2000)
  assert.deepEqual([0, 299, 300, 500, 799, 800, 5000].map(ms => clipStep(angry, ms)), [0, 0, 1, 2, 2, 3, 3])
  assert.equal(clipDelay(angry, 250), 50)
  assert.equal(clipDelay(angry, 2000), Infinity)
})

test('the anger mark shows from the stamp on, the crest all along, a baked heart takes nothing', () => {
  assert.deepEqual([0, 1, 2, 3].map(step => clipEffect(angry, step)), [null, null, 'angry', 'angry'])
  assert.equal(clipEffect(cast, 0), 'flex')
  assert.equal(clipEffect({ ...angry, effect: null }, 3), null)
})

test('a clip frame keeps the hero in its cell facing either way, and its idle steps are the hero frame', () => {
  assert.equal(clipDraw(angry, 0, 100, 50, 'right'), null)
  assert.deepEqual(clipDraw(angry, 2, 100, 50, 'right'), { sx: 128, sy: 0, width: 128, height: 128, x: 132, y: 114, flip: false })
  assert.equal(clipDraw(angry, 2, 100, 50, 'left').x, 132, 'a centred hero stays put when mirrored')
  assert.deepEqual([clipDraw(cast, 1, 0, 0, 'right').x, clipDraw(cast, 1, 0, 0, 'left').x], [-32, -32])
  assert.equal(clipDraw({ ...cast, origin: [-10, 0], width: 220 }, 0, 0, 0, 'left').x, 192 - 220 + 10)
  assert.deepEqual([clipReach(angry), clipReach(cast)], [0, 32])
})

test('only a well-formed clip on a free emote reaches the visitor; anything else plays the contract clip', () => {
  assert.deepEqual(validateEmoteClip(angry), { revision: angry.revision, sheet: angry.sheet, columns: 3, rows: 1, width: 128,
    height: 128, origin: [32, 64], frames: [null, 0, 1, 2], durations: [300, 200, 300, 1200], effect: 'mark', effectFrom: 2 })
  for (const bad of [{ v: 2 }, { revision: 'x' }, { sheet: sheet(383, 128) }, { sheet: 'nope' }, { frames: [null, 0, 1, 3] },
    { durations: [300] }, { durations: [300, 200, 300, 0] }, { effect: 'heart' }, { origin: [0] }, { columns: 0 }, { width: 600 },
    { unchanged: true, sheet: undefined }])
    assert.equal(validateEmoteClip({ ...angry, ...bad }), null, JSON.stringify(bad))
  const frame = sheet(128, 128)
  const character = { id: '12345678-1234-1234-1234-123456789abc', png,
    animation: { version: 2, frames: Array(31).fill(frame), mirroredFrames: Array(31).fill(frame) } }
  const now = Date.parse('2026-11-17T12:00:00Z')
  const visit = { id: character.id, name: 'Squire', guild: null, emote: 'angry', expiresAt: new Date(now + 60000).toISOString(), character, clip: angry }
  assert.equal(validateVisit(visit, now).clip.revision, angry.revision)
  assert.equal(Object.hasOwn(validateVisit({ ...visit, clip: { ...angry, frames: [9] } }, now), 'clip'), false)
  assert.equal(Object.hasOwn(validateVisit({ ...visit, clip: undefined }, now), 'clip'), false, 'older servers send none')
})

test('a visitor with a clip walks in, plays it once towards the host, and walks out', () => {
  const animation = { clips: { walk: { frames: [16, 17], durations: [100, 100] }, angry: { frames: [0, 12, 12, 0], durations: [400, 700, 500, 400] } } }
  assert.equal(visitDuration(animation, 'angry', angry), 1500 + 2000 + 1500)
  assert.equal(visitDuration(animation, 'angry'), 8000, 'without a clip, 5 s of the contract Angry')
  assert.equal(visitPose(animation, 1000, 112, 300, 'angry', angry).step, undefined)
  assert.deepEqual(visitPose(animation, 2200, 112, 300, 'angry', angry), { frame: 0, step: 2, state: 'angry', facing: 'left', offset: 112 })
  assert.equal(visitPose(animation, 3500, 112, 300, 'angry', angry).state, 'walk')
})

test('the hero window widens on both sides for a wide clip, keeping the hero where it stands', () => {
  const workArea = { x: 0, y: 0, width: 1440, height: 900 }
  const plain = windowRect(600, { workArea, bubble: true }), wide = windowRect(600, { workArea, bubble: true, reach: 32 })
  assert.equal(wide.width, plain.width + 64)
  assert.equal(wide.x + wide.spriteX, plain.x + plain.spriteX)
})
