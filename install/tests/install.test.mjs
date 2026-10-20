import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readlinkSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('../install.sh', import.meta.url))

test('macOS desktop installer', { skip: process.platform !== 'darwin' }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'clashofmeat-desktop-test-'))
  const routes = new Map()
  const server = createServer((request, response) => {
    const body = routes.get(request.url)
    if (body === undefined) {
      response.writeHead(404)
      response.end('missing')
    } else response.end(body)
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const localUrl = `http://127.0.0.1:${server.address().port}`
  const releases = `${localUrl}/releases`
  const home = name => {
    const path = join(root, name)
    mkdirSync(path)
    return path
  }
  const appPath = h => join(h, 'Applications', 'Clash of Meat.app')
  const binPath = h => join(h, '.local', 'bin', 'clashofmeat')
  const logPath = h => join(h, 'started.log')
  const started = h => readFileSync(logPath(h), 'utf8').trim().split('\n')

  function publish(version, { manifestVersion = version, sha256, size } = {}) {
    const fixture = join(root, `fixture-${version}`)
    const executable = join(fixture, 'Clash of Meat.app', 'Contents', 'MacOS', 'clashofmeat')
    mkdirSync(join(fixture, 'Clash of Meat.app', 'Contents', 'MacOS'), { recursive: true })
    writeFileSync(executable,
      `#!/bin/sh\nprintf '%s\\n' '${version}' > "$CLASHOFMEAT_TEST_LOG"\nprintf '%s\\n' "$@" >> "$CLASHOFMEAT_TEST_LOG"\n`,
      { mode: 0o755 })
    const tarball = join(root, `app-${version}.tar.gz`)
    const tar = spawnSync('tar', ['-czf', tarball, '-C', fixture, 'Clash of Meat.app'], { encoding: 'utf8' })
    assert.equal(tar.status, 0, tar.stderr)
    const bytes = readFileSync(tarball)
    const hash = createHash('sha256').update(bytes).digest('hex')
    const base = `/releases/download/desktop-v${version}`
    const assets = {}
    for (const arch of ['aarch64', 'x86_64']) {
      const target = `${arch}-apple-darwin`
      const file = `Clash.of.Meat_${version}_${arch}.app.tar.gz`
      assets[target] = { file, sha256: sha256 ?? hash, size: size ?? bytes.length }
      routes.set(`${base}/${file}`, bytes)
    }
    routes.set(`${base}/manifest.json`, JSON.stringify({ version: manifestVersion, assets }))
  }

  function run(h, args = [], { file = script, env = {} } = {}) {
    return new Promise((resolve, reject) => {
      const child = spawn('sh', [file, ...args], {
        env: {
          PATH: process.env.PATH,
          HOME: h,
          CLASHOFMEAT_RELEASES_URL: releases,
          CLASHOFMEAT_APPS_DIR: join(h, 'Applications'),
          CLASHOFMEAT_BIN_DIR: join(h, '.local', 'bin'),
          CLASHOFMEAT_TEST_LOG: logPath(h),
          ...env,
        },
      })
      let stdout = '', stderr = ''
      child.stdout.on('data', chunk => { stdout += chunk })
      child.stderr.on('data', chunk => { stderr += chunk })
      child.on('error', reject)
      child.on('close', status => resolve({ status, stdout, stderr }))
    })
  }

  try {
    publish('1.2.3')
    const installedHome = home('installed')
    const installed = await run(installedHome, ['--app', localUrl, '--version', '1.2.3', '--no-browser', 'two words'])
    assert.equal(installed.status, 0, installed.stderr)
    assert(existsSync(join(appPath(installedHome), 'Contents', 'MacOS', 'clashofmeat')))
    assert.equal(readlinkSync(binPath(installedHome)), join(appPath(installedHome), 'Contents', 'MacOS', 'clashofmeat'))
    assert.deepEqual(started(installedHome), ['1.2.3', 'install', '--app', localUrl, '--no-browser', 'two words'])
    assert.match(installed.stdout, /not on PATH/)
    for (const rc of ['.zshrc', '.bashrc', '.profile']) assert(!existsSync(join(installedHome, rc)))

    publish('1.2.4')
    const reinstalled = await run(installedHome, ['--version', '1.2.4', '--app', localUrl])
    assert.equal(reinstalled.status, 0, reinstalled.stderr)
    assert.deepEqual(started(installedHome), ['1.2.4', 'install', '--app', localUrl])
    assert.equal(readlinkSync(binPath(installedHome)), join(appPath(installedHome), 'Contents', 'MacOS', 'clashofmeat'))

    const refused = async (name, version, options, pattern) => {
      publish(version, options)
      const h = home(name)
      const result = await run(h, ['--app', localUrl, '--version', version])
      assert.notEqual(result.status, 0, name)
      assert.match(result.stderr, pattern, name)
      assert(!existsSync(appPath(h)), `${name}: app must not be installed`)
      assert(!existsSync(logPath(h)), `${name}: CLI must not run`)
    }
    await refused('bad-sha', '1.3.0', { sha256: '0'.repeat(64) }, /sha256/)
    await refused('bad-size', '1.3.1', { size: 1 }, /size/)
    await refused('bad-version', '1.3.2', { manifestVersion: '9.9.9' }, /version.*does not match/)

    const insecureHome = home('insecure-app')
    const insecure = await run(insecureHome, ['--app', 'http://example.com', '--version', '1.2.3'])
    assert.notEqual(insecure.status, 0)
    assert.match(insecure.stderr, /https:\/\//)
    assert(!existsSync(appPath(insecureHome)))

    const missingHome = home('missing-version')
    const missing = await run(missingHome, ['--app', localUrl])
    assert.notEqual(missing.status, 0)
    assert.match(missing.stderr, /version is required/)
    assert(!existsSync(appPath(missingHome)))

    const blockedHome = home('regular-file')
    mkdirSync(join(blockedHome, '.local', 'bin'), { recursive: true })
    writeFileSync(binPath(blockedHome), 'keep me')
    const blocked = await run(blockedHome, ['--app', localUrl, '--version', '1.2.3'])
    assert.notEqual(blocked.status, 0)
    assert.match(blocked.stderr, /regular file or directory/)
    assert.equal(readFileSync(binPath(blockedHome), 'utf8'), 'keep me')
    assert(!existsSync(appPath(blockedHome)))

    const template = readFileSync(script, 'utf8')
    assert.equal(template.split('__CLASHOFMEAT_APP_URL__').length, 2)
    assert.equal(template.split('__CLASHOFMEAT_VERSION__').length, 2)
    const personalised = join(root, 'personalised-install.sh')
    writeFileSync(personalised, template.replace('__CLASHOFMEAT_APP_URL__', localUrl).replace('__CLASHOFMEAT_VERSION__', '1.2.3'))
    const personalisedHome = home('personalised')
    const personalisedResult = await run(personalisedHome, [], { file: personalised })
    assert.equal(personalisedResult.status, 0, personalisedResult.stderr)
    assert.deepEqual(started(personalisedHome), ['1.2.3', 'install', '--app', localUrl])

    const envHome = home('env-app')
    const fromEnv = await run(envHome, ['--version', '1.2.3'], { env: { CLASHOFMEAT_APP_URL: `${localUrl}/` } })
    assert.equal(fromEnv.status, 0, fromEnv.stderr)
    assert.deepEqual(started(envHome), ['1.2.3', 'install', '--app', localUrl])
  } finally {
    await new Promise(resolve => server.close(resolve))
    rmSync(root, { recursive: true, force: true })
  }
})
