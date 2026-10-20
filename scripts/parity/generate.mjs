import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, appendFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

const providers = {
  'claude-code': { module: 'claude', scan: 'scanClaude', oversized: '.claude/projects/-tmp-demo/11111111-1111-4111-8111-111111111111.jsonl', idle: 'claude' },
  'claude-cowork': { module: 'claude', scan: 'scanCowork', oversized: 'Library/Application Support/Claude/local-agent-mode-sessions/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa/workspace/local_session/audit.jsonl', idle: 'claude' },
  codex: { module: 'codex', scan: 'scanCodex', oversized: '.codex/sessions/2026/10/01/rollout-main.jsonl', idle: 'codex' },
  gemini: { module: 'gemini', scan: 'scanGemini', oversized: '.gemini/tmp/project-hash/chats/session-2026-10-01-main.jsonl', idle: 'gemini' },
  cursor: { module: 'cursor', scan: 'scanCursor', oversized: '.clashofmeat/spool/cursor.jsonl', idle: 'cursor' },
}
const root = resolve(import.meta.dirname, '../..')
const accountId = 'aaaaaaaa-0000-4000-8000-000000000001'
const paired = '2000-01-01T00:00:00.000Z'
const fixedNow = Date.parse('2026-10-03T00:00:00.000Z')
const chunk = 4 * 1024 * 1024

function args(argv) {
  let plugin, check = false, provider
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--plugin' && argv[i + 1]) plugin = resolve(argv[++i])
    else if (argv[i] === '--provider' && argv[i + 1]) provider = argv[++i]
    else if (argv[i] === '--check') check = true
    else throw new Error(`Unknown or incomplete argument: ${argv[i]}`)
  }
  if (!plugin || !existsSync(join(plugin, 'collector', 'collector.mjs'))) throw new Error('Pass --plugin <path-to-guildbyte-plugin>')
  if (provider && !providers[provider]) throw new Error(`Unknown provider: ${provider}`)
  return { plugin, check, selected: provider ? [provider] : Object.keys(providers) }
}

function firstDifference(expected, actual) {
  if (expected === null) return 'missing expected file'
  const before = expected.split('\n'), after = actual.split('\n')
  const line = before.findIndex((value, i) => value !== after[i])
  return line < 0 ? `line count ${before.length} vs ${after.length}` : `line ${line + 1}: ${JSON.stringify(before[line] ?? '')} vs ${JSON.stringify(after[line] ?? '')}`.slice(0, 180)
}

const { plugin, check, selected } = args(process.argv.slice(2))
if (selected.length > 1) {
  let failed = false
  for (const id of selected) {
    const result = spawnSync(process.execPath, [process.argv[1], '--plugin', plugin, '--provider', id, ...(check ? ['--check'] : [])], { encoding: 'utf8' })
    if (result.stdout) process.stdout.write(result.stdout)
    if (result.stderr) process.stderr.write(result.stderr)
    if (result.error) console.error(`${id}: ${result.error.message}`)
    if (result.status !== 0) failed = true
  }
  if (failed) process.exitCode = 1
} else {
const old = relative => import(pathToFileURL(join(plugin, relative)).href)

let different = false
for (const id of selected) {
  const spec = providers[id]
  const temp = mkdtempSync(join(tmpdir(), `clashofmeat-parity-${id}-`))
  const home = join(temp, 'home')
  let db
  try {
    cpSync(join(root, 'fixtures', id, 'input'), home, { recursive: true })
    const oversized = join(home, spec.oversized)
    appendFileSync(oversized, 'x'.repeat(chunk + 128) + '\n{"type":"fixture-ignored","event":"fixture-ignored"}\n')
    Object.assign(process.env, {
      HOME: home,
      CLAUDE_CONFIG_DIR: join(home, '.claude'),
      CODEX_HOME: join(home, '.codex'),
      GEMINI_CLI_HOME: home,
      CLASHOFMEAT_HOME: join(home, '.clashofmeat'),
      XDG_CONFIG_HOME: join(home, '.config'),
    })
    const [{ openDatabase, markPaired, pairedAt }, { ensureCollectorSchema }, { createHealth }, { finalizeIdleTasks, observedTaskPayloads }] = await Promise.all([
      old('mods/clashofmeat/scripts/worker.mjs'), old('collector/lines.mjs'), old('collector/health.mjs'), old('mods/clashofmeat/scripts/task-observer.mjs'),
    ])
    db = openDatabase(join(temp, 'database'))
    ensureCollectorSchema(db)
    db.prepare('INSERT INTO accounts(id,plan,token,installation_id,pairing) VALUES (?,?,?,?,?)')
      .run(accountId, 'collector', 'fixture-token', 'fixture-installation', 'paired')
    markPaired(db, accountId, paired)
    const health = createHealth()
    const context = { accountId, since: pairedAt(db, accountId), health }
    const adapter = await old(`collector/providers/${spec.module}.mjs`)
    const remaining = adapter[spec.scan](db, context, Date.now() + 60_000, process.env, 'darwin')
    if (remaining) throw new Error(`${id} scan did not finish`)
    finalizeIdleTasks(db, spec.idle, accountId, fixedNow, (spec.idle === 'claude' || spec.idle === 'gemini' ? 10 : 30) * 60_000)
    const rows = db.prepare('SELECT id,session_id,at,key,value,kind,provider,model FROM events ORDER BY id').all()
    const tasks = observedTaskPayloads(db, accountId, 1000).sort((a, b) => a.task.id.localeCompare(b.task.id))
    const outputs = {
      'expected.json': JSON.stringify(rows, null, 2) + '\n',
      'expected-tasks.json': JSON.stringify(tasks, null, 2) + '\n',
    }
    for (const [name, content] of Object.entries(outputs)) {
      const target = join(root, 'fixtures', id, name)
      if (check) {
        const committed = existsSync(target) ? readFileSync(target, 'utf8') : null
        if (committed !== content) { different = true; console.error(`${id}/${name}: ${firstDifference(committed, content)}`) }
      } else {
        mkdirSync(dirname(target), { recursive: true })
        writeFileSync(target, content)
      }
    }
    const counters = Object.fromEntries([...health.tally].map(([name, value]) => [name, {
      files: value.files, lines: value.lines, skipped: value.skipped, errors: value.errors,
      formats: [...value.formats].sort(), versions: [...value.versions].sort(),
    }]))
    console.log(`${id}: ${rows.length} rows; adapter health ${JSON.stringify(counters)}`)
  } finally {
    db?.close()
    rmSync(temp, { recursive: true, force: true })
  }
}
if (different) process.exitCode = 1
}
