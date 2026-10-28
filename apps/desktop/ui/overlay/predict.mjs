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
