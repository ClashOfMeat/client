import assert from 'node:assert/strict'
import test from 'node:test'
import { relativeSyncTime, providerView, settingsView, updateView } from '../view.mjs'

test('update view covers optional, security, failure, and blocked states', () => {
  const base = { current: '0.1.0', available: '0.2.0' }
  assert.equal(updateView(base).line, 'v0.1.0 · up to date')
  assert.equal(updateView({ ...base, phase: 'checking' }).line, 'v0.1.0 · checking…')
  assert.equal(updateView({ ...base, phase: 'downloading' }).line, 'v0.1.0 · downloading 0.2.0…')
  assert.deepEqual(updateView({ ...base, phase: 'ready' }).action, 'Restart')
  const security = updateView({ ...base, phase: 'ready', security: true })
  assert.equal(security.line, 'v0.1.0 · ⚠ 0.2.0 ready (security fix)')
  assert.equal(security.security, true)
  const failed = updateView({ ...base, phase: 'failed', error: 'offline', blocked: true })
  assert.equal(failed.action, 'Retry')
  assert.equal(failed.error, 'offline')
  assert.equal(failed.blocked, true)
  assert.equal(updateView(base, 'minimum version required').blocked, true)
})

test('relative sync time handles absent, recent, and older timestamps', () => {
  const now = Date.parse('2026-10-20T12:00:00Z')
  assert.equal(relativeSyncTime(null, now), 'never synced')
  assert.equal(relativeSyncTime('2026-10-20T11:58:00Z', now), '2 min ago')
  assert.equal(relativeSyncTime('2026-10-20T09:00:00Z', now), '3 hr ago')
})

test('provider state prioritizes detection and wiring, and formats optional tokens', () => {
  const busy = new Set(['cursor'])
  assert.equal(providerView({ id: 'cursor', detected: true, enabled: true }, busy).state, '● active')
  assert.equal(providerView({ id: 'cursor', detected: false, wiring: 'missing' }, busy).state, '— not found')
  assert.equal(providerView({ id: 'cursor', detected: true, wiring: 'missing' }, busy).state, '⚠ hook off')
  assert.equal(providerView({ id: 'codex', detected: true, today_tokens: 310000 }, busy, true).tokens, '310k')
  assert.equal(providerView({ id: 'codex', detected: true }, busy, false).tokens, null)
})

test('settings view hides token column until a provider supplies it', () => {
  const status = { paired: true, app_url: 'https://clashofmeat.app', providers: [{ id: 'codex', name: 'Codex', detected: true, enabled: true }] }
  const snapshot = { activity: { sessions: [{ provider: 'codex', state: 'Busy' }] } }
  const view = settingsView(status, snapshot, null)
  assert.equal(view.account, 'Connected to clashofmeat.app')
  assert.equal(view.providers[0].state, '● active')
  assert.equal(view.showTokens, false)
  status.providers[0].today_tokens = 310000
  assert.equal(settingsView(status, snapshot, null).showTokens, true)
  status.today_tokens = 310000
  assert.equal(settingsView(status, snapshot, null).totalTokens, '310k')
})
