import test from 'node:test'
import assert from 'node:assert/strict'
import { COMPANION_STATES, LEAGUES, validPng, validateAnimation, validateCharacter, validateLevelUp, validateVisit } from '../validate.mjs'

const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACXBIWXMAAAPoAAAD6AG1e1JrAAAADUlEQVQImWP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg=='
const uuid = '12345678-1234-1234-1234-123456789abc'
const animation = { version: 1, frames: Array(20).fill(png), clips: Object.fromEntries(COMPANION_STATES.map(state => [state, { frames: [0, 1, 2], durations: [100, 200, 100] }])) }
const aura = { id: 'preview', heroId: 'hero', version: 1, fromLevel: 1, toLevel: 30,
  frames: Array(18).fill(png), durations: [80, ...Array(16).fill(90), 120], durationMs: 1640 }

test('older server art without Flex and Grab still gives the desktop a hero', () => {
  const clips = { ...animation.clips }
  delete clips.flex
  delete clips.grab
  for (const name of ['wave', 'laugh', 'victory']) clips[name] = { frames: [0], durations: [100] }
  const olderAnimation = { ...animation, clips }
  const character = { id: uuid, png, animation: olderAnimation }
  assert(validateAnimation(olderAnimation))
  assert(validateCharacter(character).animation)
  assert.equal(validateAnimation({ ...olderAnimation, clips: { ...clips, flex: { frames: [999], durations: [100] } } }), null)
  assert.equal(validateAnimation({ ...olderAnimation, clips: { ...clips, idle: undefined } }), null)
})

test('PNG and animation validation retain sprite limits and mirrored frames', () => {
  assert(validPng(png))
  assert.equal(validPng('bad'), false)
  assert.equal(validPng(png, 10), false)
  assert(validateAnimation(animation))
  const extended = { ...animation, frames: Array(28).fill(png), clips: { ...animation.clips, walk: { frames: [20, 21, 22, 23, 24, 25, 26, 27], durations: Array(8).fill(100) } }, mirroredFrames: { 27: png } }
  assert(validateAnimation(extended))
  assert.equal(validateAnimation({ ...extended, frames: Array(29).fill(png) }), null)
  for (const mirroredFrames of [{ 27: png }, { 2: 'bad' }, { 20: png }, [], { other: png }])
    assert.equal(validateAnimation({ ...animation, mirroredFrames }), null)
  assert.deepEqual(validateAnimation({ ...animation, mirroredFrames: { 2: png } }).mirroredFrames, { 2: png })
  assert.equal(validateAnimation({ ...animation, frames: ['bad'] }), null)
  assert.equal(validateAnimation({ ...animation, clips: { ...animation.clips, walk: { frames: [20], durations: [100] } } }), null)
  assert.equal(validateAnimation({ ...animation, clips: { ...animation.clips, talk: { frames: [1], durations: [0] } } }), null)
  assert.equal(validateAnimation({ ...animation, clips: { ...animation.clips, sit: { frames: [1, 2], durations: [100] } } }), null)
  const production = { ...animation, frames: Array(24).fill(png), clips: { ...animation.clips, walk: { frames: [16, 17, 18, 19, 20, 21, 22, 23], durations: Array(8).fill(100) } } }
  assert(validateAnimation(production))
  assert(validateAnimation({ ...production, frames: Array(28).fill(png), clips: { ...production.clips, walk: { frames: Array.from({ length: 12 }, (_, i) => 16 + i), durations: Array(12).fill(100) } } }))
  assert(validateAnimation({ ...production, frames: Array(32).fill(png) }))
  const detailed = btoa(atob(png) + '\0'.repeat(21000))
  assert(validateAnimation({ ...production, frames: Array(24).fill(detailed) }))
  assert.equal(validateAnimation({ ...production, frames: Array(32).fill(detailed.repeat(2)) }), null)
})

test('character accepts uncapped level and XP and preserves shape checks', () => {
  const character = { id: uuid, png, animation, level: 30, xp: 2_000_000, nextLevelAt: 3_000_000, canLevelUp: true, heroId: 'hero_30' }
  assert.equal(validateCharacter(character).xp, 2_000_000)
  assert.equal(validateCharacter({ ...character, xp: null, nextLevelAt: null }).nextLevelAt, null)
  assert(validateCharacter({ ...character, xp: Number.MAX_SAFE_INTEGER, nextLevelAt: 0 }))
  for (const bad of [{ level: 0 }, { level: 1.5 }, { xp: -1 }, { xp: Number.MAX_SAFE_INTEGER + 1 }, { nextLevelAt: -1 }, { canLevelUp: 1 }, { heroId: 'bad-id' }, { png: 'bad' }, { id: 'bad' }])
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
  const visit = { id: uuid, name: 'A\nB', guild: null, expiresAt: new Date(now + 60000).toISOString(), character: { id: uuid, png, animation } }
  assert.equal(validateVisit(visit, now).name, 'AB')
  assert.equal(validateVisit({ ...visit, expiresAt: new Date(now + 300001).toISOString() }, now), null)
  assert.equal(validateVisit({ ...visit, character: { id: uuid, png } }, now), null)
  assert.equal(validateVisit(visit, now).emote, 'kiss')
  assert.equal(validateVisit({ ...visit, emote: 'flex', league: 'goose' }, now).emote, 'flex')
  assert.equal(validateVisit({ ...visit, emote: 'flex', league: 'goose' }, now).league, 'goose')
  assert.equal(validateVisit({ ...visit, emote: 'wave', league: 'unknown' }, now).league, 'bronze')
  assert.equal(validateVisit({ ...visit, emote: 'wave' }, now).emote, 'kiss')
  assert.equal(validateVisit({ ...visit, emote: 'dance' }, now).emote, 'kiss')
  assert.equal(LEAGUES.length, 7)
})

test('character keeps a display name and rejects a bad one', () => {
  const character = { id: uuid, png, name: 'Framework Familiar' }
  assert.equal(validateCharacter(character).name, 'Framework Familiar')
  assert.equal(validateCharacter({ ...character, name: 'x'.repeat(81) }), null)
  assert.equal(validateCharacter({ ...character, name: 7 }), null)
})

test('character keeps its evolution stage and rejects a bad one', () => {
  const character = { id: uuid, png, evolution: 4 }
  assert.equal(validateCharacter(character).evolution, 4)
  assert.equal(validateCharacter({ ...character, evolution: 0 }), null)
  assert.equal(validateCharacter({ ...character, evolution: 2.5 }), null)
})
