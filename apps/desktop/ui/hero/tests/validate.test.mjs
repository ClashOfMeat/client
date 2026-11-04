import test from 'node:test'
import assert from 'node:assert/strict'
import { COMPANION_STATES, LEAGUES, VISIT_EMOTES, validPng, validateAnimation, validateCharacter, validateLevelUp, validateVisit } from '../validate.mjs'
import { HERO_CONTRACT } from '../hero-contract.mjs'

const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACXBIWXMAAAPoAAAD6AG1e1JrAAAADUlEQVQImWP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg=='
// Shape fixtures: validation reads the PNG header; image decoding is tested separately.
const frame = size => {
  const bytes = Buffer.from(png, 'base64')
  bytes.writeUInt32BE(size, 16)
  bytes.writeUInt32BE(size, 20)
  return bytes.toString('base64')
}
const png128 = frame(128), png192 = frame(192)
const uuid = '12345678-1234-1234-1234-123456789abc'
const animation = { version: 2, frames: Array(31).fill(png128), mirroredFrames: Array(31).fill(png128) }
const animation3 = { version: 3, frames: Array(31).fill(png192), mirroredFrames: Array(31).fill(png192) }
const aura = { id: 'preview', heroId: 'hero', version: 1, fromLevel: 1, toLevel: 30,
  frames: Array(18).fill(png), durations: [80, ...Array(16).fill(90), 120], durationMs: 1640 }

test('the hero contract: 31 frames, nine animations, three emotes, every clip inside the frames', () => {
  assert.equal(HERO_CONTRACT.version, 3)
  assert.equal(HERO_CONTRACT.frames.size, 192)
  assert.equal(HERO_CONTRACT.limits.pngBase64, 64 * 1024)
  assert.equal(HERO_CONTRACT.limits.intrinsicPngBase64, 80 * 1024)
  assert.equal(HERO_CONTRACT.limits.framesBase64, 1024 * 1024)
  assert.equal(HERO_CONTRACT.limits.intrinsicFramesBase64, 1536 * 1024)
  assert.deepEqual(COMPANION_STATES, ['idle', 'walk', 'sit', 'talk', 'kiss', 'angry', 'flex', 'grab', 'sleep'])
  assert.deepEqual(VISIT_EMOTES, ['flex', 'kiss', 'angry'])
  assert.deepEqual(Object.keys(HERO_CONTRACT.clips).sort(), [...COMPANION_STATES].sort())
  for (const [state, clip] of Object.entries(HERO_CONTRACT.clips)) {
    assert.equal(clip.frames.length, clip.durations.length, state)
    assert(clip.frames.every(index => Number.isInteger(index) && index >= 0 && index < HERO_CONTRACT.frames.total), state)
  }
  assert.deepEqual(HERO_CONTRACT.clips.walk.frames, Array.from({ length: 12 }, (_, i) => 16 + i))
  assert.deepEqual(HERO_CONTRACT.clips.flex.frames, [0, 28, 29, 30])
})

test('an animation is 31 frames each way and plays the contract clips', () => {
  assert(validPng(png))
  assert.equal(validPng('bad'), false)
  assert.equal(validPng(png, 10), false)
  const valid = validateAnimation(animation)
  assert.deepEqual(valid, { ...animation, size: 128, clips: HERO_CONTRACT.clips })
  assert.deepEqual(validateAnimation(animation3), { ...animation3, size: 192, clips: HERO_CONTRACT.clips })
  assert.equal(validateAnimation({ ...animation3, frames: animation.frames }), null, 'a 128px PNG cannot masquerade as a 192px frame')
  assert.equal(validateAnimation({ ...animation, mirroredFrames: animation3.mirroredFrames }), null, 'both facings need the same native size')
  assert.strictEqual(validateAnimation({ ...animation, clips: { idle: { frames: [99], durations: [1] } } }).clips, HERO_CONTRACT.clips,
    'the server never chooses the clips')
  for (const count of [15, 20, 28, 30, 32]) {
    assert.equal(validateAnimation({ ...animation, frames: Array(count).fill(png) }), null, `${count} frames`)
    assert.equal(validateAnimation({ ...animation, mirroredFrames: Array(count).fill(png) }), null, `${count} mirrored frames`)
  }
  for (const broken of [{ version: 1 }, { mirroredFrames: undefined }, { mirroredFrames: { 0: png } }, { frames: [...animation.frames.slice(1), 'bad'] }])
    assert.equal(validateAnimation({ ...animation, ...broken }), null, JSON.stringify(Object.keys(broken)))
  const detailed = btoa(atob(png128) + '\0'.repeat(21000))
  assert(validateAnimation({ ...animation, frames: Array(31).fill(detailed) }))
  assert.equal(validateAnimation({ ...animation, frames: Array(31).fill(detailed.repeat(2)) }), null)
})

test('lossless v3 frames may use 80 KiB each while v2 keeps its 64 KiB limit', () => {
  const large192 = btoa(atob(png192) + '\0'.repeat(52_400))
  const large128 = btoa(atob(png128) + '\0'.repeat(52_400))
  const tooLarge192 = btoa(atob(png192) + '\0'.repeat(62_000))
  assert(large192.length > HERO_CONTRACT.limits.pngBase64)
  assert(large192.length < HERO_CONTRACT.limits.intrinsicPngBase64)
  const v3Frames = [large192, ...animation3.frames.slice(1)]
  assert(validateAnimation({ ...animation3, frames: v3Frames }))
  assert.equal(validateAnimation({ ...animation, frames: [large128, ...animation.frames.slice(1)] }), null)
  assert.equal(validateAnimation({ ...animation3, frames: [tooLarge192, ...animation3.frames.slice(1)] }), null)
})

test('v3 can carry a lossless 1.3 MiB facing while v2 keeps its 1 MiB set limit', () => {
  const dense192 = btoa(atob(png192) + '\0'.repeat(32_000))
  const dense128 = btoa(atob(png128) + '\0'.repeat(32_000))
  const tooDense192 = btoa(atob(png192) + '\0'.repeat(39_000))
  const v3Frames = Array(31).fill(dense192)
  assert(v3Frames.reduce((sum, png) => sum + png.length, 0) > HERO_CONTRACT.limits.framesBase64)
  assert(v3Frames.reduce((sum, png) => sum + png.length, 0) < HERO_CONTRACT.limits.intrinsicFramesBase64)
  assert(validateAnimation({ ...animation3, frames: v3Frames }))
  assert.equal(validateAnimation({ ...animation, frames: Array(31).fill(dense128) }), null)
  assert.equal(validateAnimation({ ...animation3, frames: Array(31).fill(tooDense192) }), null)
})

test('a visitor brings the same art and one of the three emotes', () => {
  const character = { id: uuid, png, animation }
  assert.deepEqual(validateCharacter(character).animation.clips.flex, HERO_CONTRACT.clips.flex)
  assert.equal(validateCharacter({ ...character, displayScale: 1.12 }).displayScale, undefined)
  assert.equal(validateCharacter({ ...character, displayScale: Infinity }).displayScale, undefined)
  const now = Date.parse('2026-11-02T12:00:00Z')
  for (const emote of VISIT_EMOTES)
    assert.equal(validateVisit({ id: uuid, name: 'Squire', guild: null, emote, expiresAt: new Date(now + 60000).toISOString(), character }, now).emote, emote)
})

test('character accepts uncapped level and XP and preserves shape checks', () => {
  const character = { id: uuid, png, animation, level: 30, xp: 2_000_000, nextLevelAt: 3_000_000, canLevelUp: true, heroId: 'hero_30' }
  assert.equal(validateCharacter(character).xp, 2_000_000)
  assert.equal(validateCharacter({ ...character, xp: null, nextLevelAt: null }).nextLevelAt, null)
  assert(validateCharacter({ ...character, xp: Number.MAX_SAFE_INTEGER, nextLevelAt: 0 }))
  for (const bad of [{ level: 0 }, { level: 1.5 }, { xp: -1 }, { xp: Number.MAX_SAFE_INTEGER + 1 }, { nextLevelAt: -1 }, { canLevelUp: 1 }, { heroId: 'bad.id' }, { png: 'bad' }, { id: 'bad' }])
    assert.equal(validateCharacter({ ...character, ...bad }), null)
})

test('character keeps a non-negative safe level floor', () => {
  const character = { id: uuid, png, levelFloor: 1_000 }
  assert.equal(validateCharacter(character).levelFloor, 1_000)
  assert.equal(validateCharacter({ ...character, levelFloor: null }).levelFloor, null)
  for (const levelFloor of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1])
    assert.equal(validateCharacter({ ...character, levelFloor }), null)
})

test('level-up and visit validation preserve bounds without a level cap', () => {
  assert(validateLevelUp(aura))
  assert.equal(validateLevelUp({ ...aura, toLevel: 0 }), null)
  assert.equal(validateLevelUp({ ...aura, frames: Array(17).fill(png) }), null)
  assert.equal(validateLevelUp({ ...aura, durationMs: 1200 }), null)
  assert.equal(validateLevelUp({ ...aura, renderScale: 1.5 }).renderScale, 1.5)
  assert.equal(validateLevelUp({ ...aura, renderScale: 2 }), null)
  const larger = btoa(atob(png) + '\0'.repeat(59000))
  const expanded = { ...aura, renderScale: 1.5, frames: [larger, ...Array(17).fill(png)] }
  assert(validateLevelUp(expanded))
  assert.equal(validateLevelUp({ ...expanded, renderScale: 1 }), null)
  assert.equal(validateLevelUp({ ...expanded, frames: Array(18).fill(larger) }), null)
  const now = Date.parse('2026-10-20T12:00:00Z')
  const visit = { id: uuid, name: 'A\nB', guild: null, emote: 'kiss', expiresAt: new Date(now + 60000).toISOString(), character: { id: uuid, png, animation } }
  assert.equal(validateVisit(visit, now).name, 'AB')
  assert.equal(validateVisit({ ...visit, expiresAt: new Date(now + 300001).toISOString() }, now), null)
  assert.equal(validateVisit({ ...visit, character: { id: uuid, png } }, now), null)
  assert.equal(validateVisit(visit, now).emote, 'kiss')
  assert.equal(validateVisit({ ...visit, emote: 'flex', league: 'goose' }, now).emote, 'flex')
  assert.equal(validateVisit({ ...visit, emote: 'flex', league: 'goose' }, now).league, 'goose')
  assert.equal(validateVisit({ ...visit, emote: 'flex', league: 'goose', leagueRank: 12 }, now).leagueRank, 12)
  for (const leagueRank of [0, -1, 1.5, '12']) assert.equal(validateVisit({ ...visit, emote: 'flex', leagueRank }, now).leagueRank, undefined)
  assert.equal(validateVisit({ ...visit, emote: 'flex' }, now).leagueRank, undefined)
  assert.equal(validateVisit({ ...visit, league: 'unknown' }, now).league, 'bronze')
  for (const emote of [undefined, 'wave', 'dance']) assert.equal(validateVisit({ ...visit, emote }, now), null, `${emote} is not an emote`)
  assert.equal(LEAGUES.length, 7)
})

test('character keeps a display name and rejects a bad one', () => {
  const character = { id: uuid, png, name: 'Framework Familiar' }
  assert.equal(validateCharacter(character).name, 'Framework Familiar')
  assert.equal(validateCharacter({ ...character, name: 'x'.repeat(120) }).name.length, 120)
  assert.equal(validateCharacter({ ...character, name: 'x'.repeat(121) }), null)
  assert.equal(validateCharacter({ ...character, name: 7 }), null)
})

test('hero identity accepts the game definition key and name limits', () => {
  const character = { id: uuid, png, heroId: 'a-new_hero', name: 'H'.repeat(120) }
  assert.equal(validateCharacter(character).heroId, character.heroId)
  assert(validateCharacter({ ...character, heroId: 'h'.repeat(64) }))
  assert.equal(validateCharacter({ ...character, heroId: 'h'.repeat(65) }), null)
  for (const heroId of ['Hero', 'hero/name', '', 'hero name'])
    assert.equal(validateCharacter({ ...character, heroId }), null)
})

test('character keeps its evolution stage and rejects a bad one', () => {
  const character = { id: uuid, png, evolution: 4 }
  assert.equal(validateCharacter(character).evolution, 4)
  assert.equal(validateCharacter({ ...character, evolution: 0 }), null)
  assert.equal(validateCharacter({ ...character, evolution: 2.5 }), null)
})
