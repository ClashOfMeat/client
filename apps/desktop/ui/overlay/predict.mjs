// Between syncs the hero's numbers follow this machine's own activity, scored exactly as the
// server will score it. The daemon counts this machine's tokens the server's last answer did not
// include yet, by kind (input, output, cache writes, cache reads). The server's answer carries
// the day's rule (`progression.xpRule`, from the policy pinned for the player's day) and the
// day's work units (`progression.workTenths`): a day's hero XP is ⌊xpPerPower × P⌋ with
// P = ln(1 + min(work, ceiling) / pivot), work = Σ weight × tokens, and rested XP doubles each
// gain until its pool is spent (server: heroes/balance-rules.ts dailyXp, restedGain).

// The server's current defaults (content/policy.ts DEFAULT_BALANCE), for a server answer that
// does not carry its rule yet.
export const DEFAULT_XP_RULE = Object.freeze({
  inputTenths: 2,
  outputTenths: 10,
  cacheWriteTenths: 0,
  cacheReadTenths: 0,
  powerPivotTokens: 125_000,
  powerCeilingTokens: 30_000_000,
  xpPerPower: 130,
  xpPerMillionTokens: 50,
  restedPerIdleDay: 130,
  restedCap: 390,
})

const DAY_MS = 86_400_000
const DAY = /^\d{4}-\d{2}-\d{2}$/

// The rule the server sent, each missing or invalid field taken from the defaults.
export function xpRule(progression) {
  const sent = progression?.xpRule
  if (!sent || typeof sent !== 'object') return DEFAULT_XP_RULE
  const rule = {}
  for (const [key, fallback] of Object.entries(DEFAULT_XP_RULE)) {
    const value = sent[key]
    rule[key] = Number.isFinite(value) && value >= 0 ? value : fallback
  }
  if (!(rule.powerPivotTokens > 0)) rule.powerPivotTokens = DEFAULT_XP_RULE.powerPivotTokens
  return rule
}

// Work units in tenths of a token for tokens by kind ({ input, output, cacheWrite, cacheRead }).
export function workTenths(tokens, rule = DEFAULT_XP_RULE) {
  if (!tokens || typeof tokens !== 'object') return 0
  const count = value => Number.isSafeInteger(value) && value > 0 ? value : 0
  return count(tokens.input) * rule.inputTenths + count(tokens.output) * rule.outputTenths
    + count(tokens.cacheWrite) * rule.cacheWriteTenths + count(tokens.cacheRead) * rule.cacheReadTenths
}

// A day's hero XP for its work units in tenths, before rested XP: the server's dailyXp, with the
// same floating-point steps so both round alike. xpPerPower 0 is a day pinned to 0.1.7 (linear).
export function dayXp(tenths, rule = DEFAULT_XP_RULE) {
  if (!(tenths > 0)) return 0
  if (rule.xpPerPower > 0) {
    return Math.floor(rule.xpPerPower * Math.log(1 + Math.min(tenths / 10, rule.powerCeilingTokens) / rule.powerPivotTokens))
  }
  return Math.floor(tenths * rule.xpPerMillionTokens / 10_000_000)
}

// Today's token count from any field a server version sends.
export function tokenCount(data) {
  for (const value of [data?.effectiveTokens, data?.todayTokens, data?.today_tokens, data?.tokensToday, data?.tokens_today, data?.today?.tokens, data?.today?.totalTokens]) {
    if (Number.isFinite(value) && value >= 0) return value
  }
  return null
}

const addDays = (day, count) => new Date(Date.parse(`${day}T00:00:00Z`) + count * DAY_MS).toISOString().slice(0, 10)

// The server's day as of `now`: its work units in tenths (null when unknown) and its local date.
// Once `expiresAt` has passed, a new day has started at 0 that the answer does not cover yet.
export function serverDay(progression, now = Date.now()) {
  const expiresAt = Date.parse(progression?.expiresAt)
  const expired = Number.isFinite(expiresAt) && now >= expiresAt
  let day = typeof progression?.localDay === 'string' && DAY.test(progression.localDay) ? progression.localDay : null
  if (expired) return { work: 0, day: day && addDays(day, 1 + Math.floor((now - expiresAt) / DAY_MS)), expired }
  const sent = progression?.workTenths
  const tokens = tokenCount(progression)
  const work = Number.isSafeInteger(sent) && sent >= 0 ? sent : tokens === null ? null : Math.floor(tokens) * 10
  return { work, day, expired }
}

// The rested pool the next XP of `day` spends: the server banks restedPerIdleDay for each idle day
// since the last XP day, up to restedCap, on the first XP of a later day (restedGain). An older
// server sends no last XP day; its pool is then taken as it is.
export function restedPool(progression, day, rule = DEFAULT_XP_RULE) {
  const player = progression?.player
  const pool = Number.isSafeInteger(player?.restedXp) && player.restedXp > 0 ? player.restedXp : 0
  if (!player || !('lastXpDay' in player) || !day) return pool
  const last = typeof player.lastXpDay === 'string' && DAY.test(player.lastXpDay) ? player.lastXpDay : null
  if (last !== null && day <= last) return pool
  const idle = last === null ? 0 : Math.round((Date.parse(`${day}T00:00:00Z`) - Date.parse(`${last}T00:00:00Z`)) / DAY_MS) - 1
  return Math.min(rule.restedCap, pool + rule.restedPerIdleDay * Math.max(0, idle))
}

// The hero XP this machine's unsynced tokens will earn once the server counts them:
// dayXp(server day + local) − dayXp(server day), plus the rested bonus that gain spends.
export function gainedXp(progression, localPending, now = Date.now()) {
  const rule = xpRule(progression)
  const local = workTenths(localPending, rule)
  if (local <= 0) return 0
  const { work, day } = serverDay(progression, now)
  const base = dayXp((work ?? 0) + local, rule) - dayXp(work ?? 0, rule)
  if (base <= 0) return 0
  return base + Math.min(restedPool(progression, day, rule), base)
}

// The hero's XP and today's work units to show. `localPending` is this machine's unsynced
// tokens by kind, `localToday` its tokens since UTC midnight by kind (the daemon's own total,
// which covers an expired server answer or a missing heartbeat baseline).
export function predict(character, progression, localPending, localToday, now = Date.now()) {
  const rule = xpRule(progression)
  let xp = character?.xp
  if (Number.isSafeInteger(xp)) {
    const gained = gainedXp(progression, localPending, now)
    // Levelling up is the server's call, so a guess stops one XP short of the next level.
    const cap = Number.isSafeInteger(character.nextLevelAt) ? character.nextLevelAt - 1 : Infinity
    if (gained > 0) xp = Math.max(xp, Math.min(xp + gained, cap))
  }
  const { work, expired } = serverDay(progression, now)
  const pending = workTenths(localPending, rule)
  const today = workTenths(localToday, rule)
  const tenths = expired ? today : work === null ? (today || null) : Math.max(work + pending, today)
  return { xp, tokens: tenths === null ? null : Math.floor(tenths / 10) }
}

// Whether this machine's tokens already pay for the next level. The bar then waits one XP short
// of it (see predict) until the server confirms, so the overlay asks for a sync instead of
// waiting for the next heartbeat.
export function levelDue(character, progression, localPending, now = Date.now()) {
  if (!Number.isSafeInteger(character?.xp) || !Number.isSafeInteger(character.nextLevelAt)) return false
  if (character.xp >= character.nextLevelAt) return false
  const gained = gainedXp(progression, localPending, now)
  return gained > 0 && character.xp + gained >= character.nextLevelAt
}

// The XP to show, given what was shown last ({ id, level, serverXp, xp }). On the same hero and
// level the number never goes down while the server's own XP has not: a guess that shrinks (a
// daemon restart, a refused observation, a heartbeat still decoding) keeps the last number
// until the server catches up.
export function steadyXp(shown, character, xp) {
  const next = { id: character?.id, level: character?.level, serverXp: character?.xp, xp }
  if (!shown || !Number.isSafeInteger(xp) || !Number.isSafeInteger(shown.xp)) return next
  if (shown.id !== next.id || shown.level !== next.level) return next
  if (Number.isSafeInteger(shown.serverXp) && next.serverXp < shown.serverXp) return next
  return { ...next, xp: Math.max(xp, shown.xp) }
}
