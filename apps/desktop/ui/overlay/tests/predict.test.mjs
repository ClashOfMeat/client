import test from 'node:test'
import assert from 'node:assert/strict'
import { predict, tokenCount } from '../predict.mjs'

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
