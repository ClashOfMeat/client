import test from 'node:test'
import assert from 'node:assert/strict'
import { predict, tokenCount, levelDue, steadyXp, gainedXp, dayXp, workTenths, xpRule, DEFAULT_XP_RULE } from '../predict.mjs'

// Every expected XP below was computed with the server's own functions (clashofmeat-app
// packages/core/src/heroes/balance-rules.ts dailyXp and restedGain, DEFAULT_BALANCE).
const RULE = {
  inputTenths: 2, outputTenths: 10, cacheWriteTenths: 0, cacheReadTenths: 0,
  powerPivotTokens: 125_000, powerCeilingTokens: 30_000_000, xpPerPower: 130, xpPerMillionTokens: 50,
  restedPerIdleDay: 130, restedCap: 390,
}
const NOW = Date.parse('2026-11-13T15:00:00Z')
const EXPIRES = '2026-11-14T00:00:00Z'
const day = (workTenths, player = { restedXp: 0, lastXpDay: '2026-11-13' }) => ({
  localDay: '2026-11-13', expiresAt: EXPIRES, effectiveTokens: Math.floor(workTenths / 10), workTenths, xpRule: RULE, player,
})
const character = { xp: 1000, nextLevelAt: 2000 }
const out = output => ({ input: 0, output, cacheWrite: 0, cacheRead: 0 })

test('cache-only activity earns nothing', () => {
  const cache = { input: 0, output: 0, cacheWrite: 1_000_000, cacheRead: 5_000_000 }
  assert.equal(gainedXp(day(0), cache, NOW), 0)
  assert.deepEqual(predict(character, day(0), cache, cache, NOW), { xp: 1000, tokens: 0 })
})

test('125k output on an empty day earns 90, as on the server', () => {
  assert.equal(gainedXp(day(0), out(125_000), NOW), 90)
  assert.deepEqual(predict(character, day(0), out(125_000), out(125_000), NOW), { xp: 1090, tokens: 125_000 })
})

test('the same tokens late in a heavy day earn less', () => {
  assert.equal(gainedXp(day(50_000_000), out(125_000), NOW), 3)
  assert.equal(gainedXp(day(0), out(1_000_000), NOW), 285)
  assert.equal(gainedXp(day(100_000_000), out(1_000_000), NOW), 12)
})

test('input counts a fifth and the exact server tenths are used', () => {
  assert.equal(workTenths({ input: 100_000, output: 50_000, cacheWrite: 7, cacheRead: 2_000_000 }, RULE), 700_000)
  // 1,250,005 tenths: effectiveTokens alone would drop the half token.
  assert.equal(gainedXp(day(1_250_005), { input: 100_000, output: 50_000, cacheWrite: 0, cacheRead: 2_000_000 }, NOW), 32)
  assert.equal(predict(character, day(1_250_005), { input: 3, output: 0 }, null, NOW).tokens, 125_001)
})

test('tokens past the ceiling add nothing', () => {
  assert.equal(gainedXp(day(299_000_000), out(1_000_000), NOW), 1)
  assert.equal(gainedXp(day(300_000_000), out(1_000_000), NOW), 0)
  assert.equal(gainedXp(day(0), out(40_000_000), NOW), 713)
})

test('rested XP doubles the gain while its pool lasts, banked like the server', () => {
  // Same day: the pool covers 40 of the 90.
  assert.equal(gainedXp(day(0, { restedXp: 40, lastXpDay: '2026-11-13' }), out(125_000), NOW), 130)
  // Last XP three days ago: two idle days bank 260 first, so the whole gain doubles.
  assert.equal(gainedXp(day(0, { restedXp: 40, lastXpDay: '2026-11-10' }), out(125_000), NOW), 180)
  // The bank stops at the cap (390): a full day's 713 gets 390 more, not 713.
  assert.equal(gainedXp(day(0, { restedXp: 380, lastXpDay: '2026-11-01' }), out(40_000_000), NOW), 1103)
  // An older server sends no last XP day: its pool is spent as it is.
  assert.equal(gainedXp(day(0, { restedXp: 40 }), out(125_000), NOW), 130)
})

test('a guess never levels the hero up and never goes backwards', () => {
  const near = { xp: 1950, nextLevelAt: 2000 }
  assert.equal(predict(near, day(0), out(125_000), null, NOW).xp, 1999)
  assert.equal(predict({ xp: 2100, nextLevelAt: 2000 }, day(0), out(125_000), null, NOW).xp, 2100)
  assert.equal(predict({ xp: 5000, nextLevelAt: null }, day(0), out(125_000), null, NOW).xp, 5090)
  assert.equal(predict(character, day(0), { output: -5 }, null, NOW).xp, 1000)
  assert.equal(predict(null, day(0), out(2), null, NOW).xp, undefined)
})

test('a level is due when local tokens pay for it, not before and not once the server has it', () => {
  assert.equal(levelDue({ xp: 1910, nextLevelAt: 2000 }, day(0), out(125_000), NOW), true)
  assert.equal(levelDue({ xp: 1909, nextLevelAt: 2000 }, day(0), out(125_000), NOW), false)
  assert.equal(levelDue({ xp: 1999, nextLevelAt: 2000 }, day(0), { cacheRead: 9_000_000 }, NOW), false)
  assert.equal(levelDue({ xp: 2000, nextLevelAt: 2000 }, day(0), out(125_000), NOW), false)
  assert.equal(levelDue({ xp: 5000, nextLevelAt: null }, day(0), out(125_000), NOW), false)
})

test('an older server answer falls back to the current default rule, not 50 per million', () => {
  const old = { effectiveTokens: 125_000, expiresAt: EXPIRES }
  assert.equal(xpRule(old), DEFAULT_XP_RULE)
  assert.equal(gainedXp(old, out(125_000), NOW), 52)
  assert.equal(gainedXp(old, { input: 0, output: 0, cacheWrite: 2_000_000, cacheRead: 0 }, NOW), 0)
  assert.equal(predict(character, old, out(125_000), null, NOW).tokens, 250_000)
  // A partial rule keeps the defaults for what it lacks.
  assert.equal(xpRule({ xpRule: { xpPerPower: 200 } }).outputTenths, 10)
})

test('a day pinned to 0.1.7 keeps its linear rule', () => {
  const legacy = { ...RULE, inputTenths: 10, cacheWriteTenths: 10, cacheReadTenths: 1, powerPivotTokens: 500_000, powerCeilingTokens: 1_000_000_000, xpPerPower: 0, restedPerIdleDay: 100, restedCap: 300 }
  assert.equal(gainedXp({ ...day(0), xpRule: legacy }, { input: 0, output: 2_000_000, cacheWrite: 0, cacheRead: 1_000_000 }, NOW), 105)
  assert.equal(dayXp(0, legacy), 0)
})

test('after the server day ends, the new day starts at 0 and banks rested XP', () => {
  const late = Date.parse('2026-11-14T00:00:10Z')
  const yesterday = day(50_000_000, { restedXp: 100, lastXpDay: '2026-11-12' })
  // The new day is 11-14: 11-13 earned nothing, so one idle day banks 130 first.
  assert.equal(gainedXp(yesterday, out(125_000), late), 180)
  assert.equal(gainedXp({ ...yesterday, player: { restedXp: 0, lastXpDay: '2026-11-13' } }, out(125_000), late), 90)
  // Two days late: two idle days bank, still capped by the gain.
  assert.equal(gainedXp(yesterday, out(125_000), late + 86_400_000), 180)
  // Today's count is this machine's own day total until a fresh answer arrives.
  assert.equal(predict(character, yesterday, out(125_000), out(400_000), late).tokens, 400_000)
  assert.equal(predict(character, yesterday, null, null, late).tokens, 0)
})

test('today is one unit: server work units plus this machine’s unsynced work units', () => {
  assert.equal(predict(character, day(5_000_000), { input: 50_000, output: 10_000, cacheWrite: 0, cacheRead: 9_000_000 }, null, NOW).tokens, 520_000)
  // The daemon's own day total wins when it is higher (no heartbeat baseline yet).
  assert.equal(predict(character, day(5_000_000), null, out(600_000), NOW).tokens, 600_000)
})

test('the token count reads the field the server sends, and stays hidden without one', () => {
  assert.equal(tokenCount({ effectiveTokens: 12 }), 12)
  assert.equal(tokenCount({ todayTokens: 7 }), 7)
  assert.equal(tokenCount({}), null)
  assert.equal(predict(character, null, out(500), null, NOW).tokens, null)
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
