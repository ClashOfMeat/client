// Between syncs the hero's numbers follow this machine's own activity. The daemon counts the
// effective tokens the server's last answer did not include yet (input, output and cache writes,
// plus a tenth of cache reads, as the server scores them). Its day total also covers an expired
// server answer when a daemon restart or offline server leaves no heartbeat baseline.
export const XP_PER_MILLION_TOKENS = 50

// Today's token count from any field a server version sends.
export function tokenCount(data) {
  for (const value of [data?.effectiveTokens, data?.todayTokens, data?.today_tokens, data?.tokensToday, data?.tokens_today, data?.today?.tokens, data?.today?.totalTokens]) {
    if (Number.isFinite(value) && value >= 0) return value
  }
  return null
}

export function predict(character, progression, localTokens, localTodayTokens = 0, now = Date.now()) {
  const local = Number.isSafeInteger(localTokens) && localTokens > 0 ? localTokens : 0
  const today = Number.isSafeInteger(localTodayTokens) && localTodayTokens > 0 ? localTodayTokens : 0
  const server = tokenCount(progression)
  const expiresAt = Date.parse(progression?.expiresAt)
  const expired = Number.isFinite(expiresAt) && now >= expiresAt
  let xp = character?.xp
  if (Number.isSafeInteger(xp) && local > 0) {
    const gained = Math.floor(local * XP_PER_MILLION_TOKENS / 1_000_000)
    // Levelling up is the server's call, so a guess stops one XP short of the next level.
    const cap = Number.isSafeInteger(character.nextLevelAt) ? character.nextLevelAt - 1 : Infinity
    xp = Math.max(xp, Math.min(xp + gained, cap))
  }
  return { xp, tokens: expired ? today : server === null ? (today || null) : Math.max(server + local, today) }
}

// Whether this machine's tokens already pay for the next level. The bar then waits one XP short
// of it (see predict) until the server confirms, so the overlay asks for a sync instead of
// waiting for the next heartbeat.
export function levelDue(character, localTokens) {
  if (!Number.isSafeInteger(character?.xp) || !Number.isSafeInteger(character.nextLevelAt)) return false
  if (!Number.isSafeInteger(localTokens) || localTokens <= 0 || character.xp >= character.nextLevelAt) return false
  return character.xp + Math.floor(localTokens * XP_PER_MILLION_TOKENS / 1_000_000) >= character.nextLevelAt
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
