import { formatTokens } from '../overlay/layout.mjs'

export function relativeSyncTime(value, now = Date.now()) {
  if (!value) return 'never synced'
  const then = typeof value === 'number' ? value : Date.parse(value)
  if (!Number.isFinite(then)) return 'sync time unavailable'
  const minutes = Math.max(0, Math.floor((now - then) / 60_000))
  if (minutes < 1) return 'just now'
  if (minutes < 60) return `${minutes} min ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} hr ago`
  return `${Math.floor(hours / 24)} d ago`
}

function busyProviders(snapshot) {
  return new Set((snapshot?.activity?.sessions ?? [])
    .filter(session => String(session.state).toLowerCase() === 'busy')
    .map(session => String(session.provider ?? session.provider_id ?? '').toLowerCase()))
}

export function providerView(provider, busy = new Set(), showTokens = false) {
  const id = String(provider.id ?? '')
  let state, tone
  if (!provider.detected) { state = '— not found'; tone = 'missing' }
  else if (provider.wiring === 'missing') { state = '⚠ hook off'; tone = 'warning' }
  else if (busy.has(id.toLowerCase()) || busy.has(String(provider.name ?? '').toLowerCase())) { state = '● active'; tone = 'active' }
  else { state = '○ idle'; tone = 'idle' }
  const tokens = provider.today_tokens ?? provider.todayTokens
  return { id, name: provider.name ?? id, enabled: provider.enabled !== false,
    state, tone, fix: provider.wiring === 'missing',
    limits: (provider.limits ?? []).map(limit => `${limit.key === 'five_hour_percent' ? '5h' : '7d'} ${limit.used}%`).join(' · ') || 'limits unavailable',
    tokens: provider.tokens_available === false ? 'not available' : showTokens && Number.isFinite(tokens) ? formatTokens(tokens) : null }
}

export function updateView(update = {}, daemonBlocked = false) {
  const phase = update.phase ?? 'up to date'
  const current = update.current ?? '0.1.0'
  const available = update.available
  const blocked = Boolean(update.blocked || daemonBlocked)
  const ready = phase === 'ready'
  const failed = phase === 'failed'
  let line = `v${current} · up to date`
  if (phase === 'checking') line = `v${current} · checking…`
  if (phase === 'downloading') line = `v${current} · downloading ${available ?? 'update'}…`
  if (ready) line = `v${current} · ${update.security ? '⚠ ' : ''}${available ?? 'update'} ready${update.security ? ' (security fix)' : ''}`
  if (failed) line = `v${current} · update failed`
  if (phase === 'installing') line = `v${current} · installing update…`
  return { line, blocked, ready, failed, security: ready && Boolean(update.security),
    action: ready ? 'Restart' : failed ? 'Retry' : null,
    error: failed ? update.error ?? 'Could not update. Try again.' : null }
}

export function settingsView(status, snapshot, heartbeat, { connected = true, now = Date.now(), updater = {} } = {}) {
  const paired = connected && status?.paired === true
  const providers = Array.isArray(status?.providers) ? status.providers : []
  const showTokens = providers.some(provider => Number.isFinite(provider.today_tokens ?? provider.todayTokens))
  let host = 'clashofmeat.app'
  try { host = new URL(status?.app_url ?? 'https://clashofmeat.app').host } catch { /* use product host */ }
  return {
    connected, paired,
    account: !connected ? "Clash of Meat isn't running" : paired
      ? `Connected to ${host}` : 'Not connected',
    sync: relativeSyncTime(status?.last_sync_at, now),
    providers: providers.map(provider => providerView(provider, busyProviders(snapshot), showTokens)),
    showTokens,
    totalTokens: status?.today_tokens != null ? formatTokens(
      typeof status.today_tokens === 'number' ? status.today_tokens
        : Object.values(status.today_tokens).reduce((sum, value) => sum + (Number.isFinite(value) ? value : 0), 0)
    ) : null,
    update: updateView(updater, status?.blocked),
    hero: heartbeat ?? snapshot?.hero ?? null,
  }
}
