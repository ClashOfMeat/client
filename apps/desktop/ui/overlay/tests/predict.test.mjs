import test from 'node:test'
import assert from 'node:assert/strict'
import { predict, tokenCount, levelDue, steadyXp } from '../predict.mjs'

const character = { xp: 1000, nextLevelAt: 1200 }

test('local tokens move the count and the XP bar until the next heartbeat', () => {
  assert.deepEqual(predict(character, { effectiveTokens: 5_000_000 }, 2_000_000), { xp: 1100, tokens: 7_000_000 })
  assert.deepEqual(predict(character, { effectiveTokens: 5_000_000 }, 0), { xp: 1000, tokens: 5_000_000 })
  assert.deepEqual(predict(character, { effectiveTokens: 5_000_000 }, undefined), { xp: 1000, tokens: 5_000_000 })
})

test('a guess never levels the hero up and never goes backwards', () => {
  assert.equal(predict(character, {}, 100_000_000).xp, 1199)
  assert.equal(predict({ xp: 1300, nextLevelAt: 1200 }, {}, 1_000_000).xp, 1300)
  assert.equal(predict({ xp: 5000, nextLevelAt: null }, {}, 1_000_000).xp, 5050)
  assert.equal(predict(character, {}, -5).xp, 1000)
})

test('the token count reads the field the server sends, and stays hidden without one', () => {
  assert.equal(tokenCount({ effectiveTokens: 12 }), 12)
  assert.equal(tokenCount({ todayTokens: 7 }), 7)
  assert.equal(tokenCount({}), null)
  assert.equal(predict(character, null, 500).tokens, null)
  assert.equal(predict(null, { effectiveTokens: 1 }, 2).xp, undefined)
})

test('expired server totals use this machine’s effective tokens for the new day', () => {
  const yesterday = { effectiveTokens: 0, expiresAt: '2026-10-28T00:00:00Z' }
  const now = Date.parse('2026-10-28T10:00:00Z')
  assert.equal(predict(character, yesterday, 0, 3_500_000, now).tokens, 3_500_000)
  assert.equal(predict(character, yesterday, 0, 0, now).tokens, 0)
  assert.equal(predict(character, { effectiveTokens: 5_000_000 }, 0, 6_000_000, now).tokens, 6_000_000)
})

test('the shown XP never drops on the same hero and level unless the server XP does', () => {
  const hero = { id: 'h', level: 3, xp: 1000 }
  const shown = steadyXp(null, hero, 1040)
  assert.equal(shown.xp, 1040)
  // The guess shrank (a daemon restart): the number holds.
  assert.equal(steadyXp(shown, hero, 1000).xp, 1040)
  // The server caught up part of the way: still no drop, then growth again.
  assert.equal(steadyXp(shown, { ...hero, xp: 1020 }, 1025).xp, 1040)
  assert.equal(steadyXp(shown, { ...hero, xp: 1020 }, 1060).xp, 1060)
  // A real loss on the server, a new level or another hero shows as it is.
  assert.equal(steadyXp(shown, { ...hero, xp: 900 }, 900).xp, 900)
  assert.equal(steadyXp(shown, { ...hero, level: 4, xp: 1010 }, 1010).xp, 1010)
  assert.equal(steadyXp(shown, { ...hero, id: 'other', xp: 5 }, 5).xp, 5)
})

test('a level is due when local tokens pay for it, not before and not once the server has it', () => {
  assert.equal(levelDue(character, 4_000_000), true)
  assert.equal(levelDue(character, 3_980_000), false)
  assert.equal(levelDue(character, 0), false)
  assert.equal(levelDue({ xp: 1200, nextLevelAt: 1200 }, 4_000_000), false)
  assert.equal(levelDue({ xp: 5000, nextLevelAt: null }, 4_000_000), false)
})
