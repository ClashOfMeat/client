import { settingsView } from './settings/view.mjs'

const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event
const $ = id => document.getElementById(id)
let daemon = { connected: false, status: null, snapshot: null, heartbeat: null }
let hero = { show: true, screen: 'main', still: false }
let displays = []
let device = 'This Mac'
let updater = {}

function error(message) {
  $('error').textContent = String(message)
  $('error').hidden = !message
}

async function request(method, params = {}) { return invoke('daemon_request', { method, params }) }

function render() {
  const view = settingsView(daemon.status, daemon.snapshot, daemon.heartbeat, { connected: daemon.connected, updater })
  $('update-required').hidden = !view.update.blocked
  for (const section of document.querySelectorAll('main > section:not(#update-required)')) section.hidden = view.update.blocked
  $('required-install').hidden = !view.update.ready
  $('required-retry').hidden = !view.update.failed
  $('required-error').hidden = !view.update.error
  $('required-error').textContent = view.update.error ?? ''
  $('error').hidden = view.update.blocked || !$('error').textContent
  $('account-status').textContent = view.account
  const pairedAt = daemon.status?.paired_at
  const since = pairedAt && Number.isFinite(Date.parse(pairedAt))
    ? ` · since ${new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short' }).format(new Date(pairedAt))}` : ''
  $('device').textContent = view.paired ? `${device}${since}` : ''
  $('connect').hidden = view.paired
  $('connect').disabled = !daemon.connected
  $('sync').hidden = !view.paired
  $('last-sync').textContent = view.paired ? `last sync ${view.sync}` : ''
  $('about-line').textContent = view.update.line
  $('about-line').classList.toggle('security', view.update.security)
  $('about-action').hidden = !view.update.action
  $('about-action').textContent = view.update.action ?? ''
  $('today-total').hidden = view.totalTokens === null
  $('today-total').textContent = view.totalTokens === null ? '' : `today ${view.totalTokens} tokens`
  $('show-hero').checked = hero.show
  $('stay-still').checked = hero.still
  const select = $('screen')
  select.querySelectorAll('option[data-display]').forEach(option => option.remove())
  for (const display of displays) {
    const option = document.createElement('option')
    option.value = display.id
    option.textContent = display.name
    option.dataset.display = 'true'
    select.append(option)
  }
  select.value = hero.screen
  const list = $('providers')
  list.replaceChildren()
  for (const provider of view.providers) {
    const row = document.createElement('div')
    row.className = 'provider'
    const check = document.createElement('input')
    check.type = 'checkbox'
    check.checked = provider.enabled
    check.setAttribute('aria-label', `Enable ${provider.name}`)
    check.addEventListener('change', async () => {
      try { await request('set_provider_enabled', { id: provider.id, enabled: check.checked }); await refresh() }
      catch (reason) { check.checked = !check.checked; error(reason) }
    })
    const name = document.createElement('span')
    name.className = 'provider-name'
    name.textContent = provider.name
    const state = document.createElement('span')
    state.className = `provider-state ${provider.tone}`
    state.textContent = provider.state
    row.append(check, name, state)
    if (view.showTokens) {
      const tokens = document.createElement('span')
      tokens.className = 'provider-tokens'
      tokens.textContent = provider.tokens ?? ''
      row.append(tokens)
    }
    if (provider.fix) {
      const fix = document.createElement('button')
      fix.textContent = 'Fix'
      fix.addEventListener('click', async () => {
        try { error(''); await request('wire_provider', { id: provider.id }); await refresh() }
        catch (reason) { error(`${provider.name}: ${reason}`) }
      })
      row.append(fix)
    }
    list.append(row)
  }
}

async function refresh() {
  const midnight = new Date()
  midnight.setHours(0, 0, 0, 0)
  try { daemon.status = await request('status', { today_since: midnight.toISOString() }); daemon.connected = true; render() }
  catch (reason) { daemon.connected = false; error(reason); render() }
}

$('sync').addEventListener('click', async () => {
  try { error(''); await request('sync_now'); await refresh() } catch (reason) { error(reason) }
})
$('connect').addEventListener('click', async () => {
  try {
    error('')
    const result = await request('pair_start')
    $('pair-code').replaceChildren()
    const label = document.createElement('span')
    label.textContent = 'Enter code '
    const code = document.createElement('strong')
    code.textContent = result.code ?? ''
    $('pair-code').append(label, code)
    $('pair-code').hidden = false
    await invoke('open_pair_link', { link: result.link })
  } catch (reason) { error(reason) }
})
$('show-hero').addEventListener('change', async event => {
  try { hero = await invoke('set_hero_settings', { show: event.target.checked, screen: hero.screen, still: hero.still }) }
  catch (reason) { error(reason); render() }
})
$('stay-still').addEventListener('change', async event => {
  try { hero = await invoke('set_hero_settings', { show: hero.show, screen: hero.screen, still: event.target.checked }) }
  catch (reason) { error(reason); render() }
})
$('screen').addEventListener('change', async event => {
  try { hero = await invoke('set_hero_settings', { show: hero.show, screen: event.target.value, still: hero.still }) }
  catch (reason) { error(reason); render() }
})

async function updateAction() {
  try {
    error('')
    if (updater.phase === 'ready') await invoke('update_restart')
    else await invoke('update_check')
  } catch (reason) { error(reason) }
}
$('about-action').addEventListener('click', updateAction)
$('required-install').addEventListener('click', updateAction)
$('required-retry').addEventListener('click', updateAction)

const initial = await invoke('desktop_state')
daemon = initial.daemon
hero = initial.hero
displays = initial.displays
device = initial.device
updater = initial.updater ?? {}
render()
await listen('daemon', ({ payload }) => {
  if (payload.kind === 'snapshot') daemon.snapshot = payload.data
  if (payload.kind === 'status') {
    if (typeof payload.data.connected === 'boolean') daemon.connected = payload.data.connected
    else daemon.status = payload.data
  }
  if (payload.kind === 'event') {
    const { event, data } = payload.data
    if (event === 'activity' && daemon.snapshot) daemon.snapshot.activity = data
    if (event === 'heartbeat') daemon.heartbeat = data
    if (event === 'paired') { $('pair-code').hidden = true; void refresh() }
    if (event === 'pair_failed') { $('pair-code').hidden = true; error(data?.reason ?? 'Pairing failed') }
  }
  render()
})
await listen('hero-settings', ({ payload }) => { hero = payload; render() })
await listen('updater', ({ payload }) => { updater = payload; render() })
setInterval(render, 60_000)
void refresh()
