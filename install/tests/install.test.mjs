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
  // A stand-in minisign: it checks the key and the decoded signature it is given, and exits with
  // FAKE_MINISIGN_STATUS. The real one is never on the test PATH.
  const fakeBin = join(root, 'fake-bin')
  mkdirSync(fakeBin)
  const updaterKey = Buffer.from(readFileSync(new URL('../../apps/desktop/src-tauri/updater.pub', import.meta.url), 'utf8'), 'base64')
    .toString('utf8').trim().split('\n')[1]
  writeFileSync(join(fakeBin, 'minisign'), `#!/bin/sh
key='' sig=''
while [ $# -gt 0 ]; do case "$1" in -P) key=$2; shift ;; -x) sig=$2; shift ;; esac; shift; done
[ "$key" = '${updaterKey}' ] || exit 9
grep -q 'fake release signature' "$sig" || exit 8
exit "\${FAKE_MINISIGN_STATUS:-0}"
`, { mode: 0o755 })

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
      routes.set(`${base}/${file}.sig`, Buffer.from('untrusted comment: fake release signature\n').toString('base64'))
    }
    routes.set(`${base}/manifest.json`, JSON.stringify({ version: manifestVersion, assets }))
  }

  function run(h, args = [], { file = script, env = {} } = {}) {
    return new Promise((resolve, reject) => {
      const child = spawn('sh', [file, ...args], {
        env: {
          PATH: `${fakeBin}:${process.env.PATH}`,
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
    assert.match(installed.stdout, /release signature/)
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

    const unsignedHome = home('bad-signature')
    const unsigned = await run(unsignedHome, ['--app', localUrl, '--version', '1.2.3'], { env: { FAKE_MINISIGN_STATUS: '1' } })
    assert.notEqual(unsigned.status, 0)
    assert.match(unsigned.stderr, /not signed with the Clash of Meat release key/)
    assert(!existsSync(appPath(unsignedHome)))
    assert(!existsSync(logPath(unsignedHome)))

    // Without minisign, the checksum still guards the download and the installer says how to add the signature check.
    const noVerifierHome = home('no-minisign')
    const noVerifier = await run(noVerifierHome, ['--app', localUrl, '--version', '1.2.3'], { env: { PATH: '/usr/bin:/bin' } })
    assert.equal(noVerifier.status, 0, noVerifier.stderr)
    assert.match(noVerifier.stdout, /install minisign/)
    assert.deepEqual(started(noVerifierHome), ['1.2.3', 'install', '--app', localUrl])

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

// The Linux branch, run on any Unix through CLASHOFMEAT_TEST_UNAME (only the OS name is faked).
test('Linux CLI installer', { skip: process.platform === 'win32' }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'clashofmeat-linux-test-'))
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
  const arch = process.arch === 'arm64' ? 'aarch64' : 'x86_64'
  const home = name => {
    const path = join(root, name)
    mkdirSync(path)
    return path
  }
  const exePath = h => join(h, '.local', 'share', 'clashofmeat', 'bin', 'clashofmeat')
  const binPath = h => join(h, '.local', 'bin', 'clashofmeat')
  const logPath = h => join(h, 'started.log')
  const started = h => readFileSync(logPath(h), 'utf8').trim().split('\n')

  function publish(version, { sha256, named } = {}) {
    const fixture = join(root, `fixture-${version}`)
    mkdirSync(fixture)
    writeFileSync(join(fixture, 'clashofmeat'),
      `#!/bin/sh\nprintf '%s\\n' '${version}' > "$CLASHOFMEAT_TEST_LOG"\nprintf '%s\\n' "$@" >> "$CLASHOFMEAT_TEST_LOG"\n`,
      { mode: 0o755 })
    const tarball = join(root, `cli-${version}.tar.gz`)
    const tar = spawnSync('tar', ['-czf', tarball, '-C', fixture, 'clashofmeat'], { encoding: 'utf8' })
    assert.equal(tar.status, 0, tar.stderr)
    const bytes = readFileSync(tarball)
    const file = `clashofmeat_${version}_linux-${arch}.tar.gz`
    const base = `/releases/download/desktop-v${version}`
    routes.set(`${base}/${file}`, bytes)
    routes.set(`${base}/${file}.sha256`, `${sha256 ?? createHash('sha256').update(bytes).digest('hex')}  ${named ?? file}\n`)
  }

  function run(h, args = [], env = {}) {
    return new Promise((resolve, reject) => {
      const child = spawn('sh', [script, ...args], {
        env: {
          // No minisign on this PATH: the checksum path is what runs here (the macOS test covers the signature).
          PATH: '/usr/bin:/bin',
          HOME: h,
          CLASHOFMEAT_TEST_UNAME: 'Linux',
          CLASHOFMEAT_RELEASES_URL: `${localUrl}/releases`,
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
    publish('2.0.0')
    const h = home('installed')
    const installed = await run(h, ['--app', localUrl, '--version', '2.0.0', '--no-connect'])
    assert.equal(installed.status, 0, installed.stderr)
    assert.equal(readlinkSync(binPath(h)), exePath(h))
    assert.deepEqual(started(h), ['2.0.0', 'install', '--app', localUrl, '--no-connect'])
    assert.match(installed.stdout, /Installed Clash of Meat 2\.0\.0/)
    assert.match(installed.stdout, /install minisign \(apt install minisign/)
    assert.match(installed.stdout, /not on PATH/)

    publish('2.0.1')
    const upgraded = await run(h, ['--app', localUrl, '--version', '2.0.1'])
    assert.equal(upgraded.status, 0, upgraded.stderr)
    assert.deepEqual(started(h), ['2.0.1', 'install', '--app', localUrl])

    // XDG_DATA_HOME moves the CLI, as `clashofmeat update` expects.
    const xdg = home('xdg')
    const moved = await run(xdg, ['--app', localUrl, '--version', '2.0.1'], { XDG_DATA_HOME: join(xdg, 'data') })
    assert.equal(moved.status, 0, moved.stderr)
    assert.equal(readlinkSync(binPath(xdg)), join(xdg, 'data', 'clashofmeat', 'bin', 'clashofmeat'))

    const refused = async (name, version, options, pattern) => {
      publish(version, options)
      const other = home(name)
      const result = await run(other, ['--app', localUrl, '--version', version])
      assert.notEqual(result.status, 0, name)
      assert.match(result.stderr, pattern, name)
      assert(!existsSync(exePath(other)), `${name}: CLI must not be installed`)
      assert(!existsSync(logPath(other)), `${name}: CLI must not run`)
    }
    await refused('bad-sha', '2.1.0', { sha256: '0'.repeat(64) }, /sha256 does not match/)
    await refused('wrong-name', '2.1.1', { named: 'other.tar.gz' }, /invalid checksum file/)
    const missingHome = home('no-build')
    const missing = await run(missingHome, ['--app', localUrl, '--version', '9.9.9'])
    assert.notEqual(missing.status, 0)
    assert.match(missing.stderr, /may have no Linux build/)

    const blockedHome = home('regular-file')
    mkdirSync(join(blockedHome, '.local', 'bin'), { recursive: true })
    writeFileSync(binPath(blockedHome), 'keep me')
    const blocked = await run(blockedHome, ['--app', localUrl, '--version', '2.0.0'])
    assert.notEqual(blocked.status, 0)
    assert.match(blocked.stderr, /regular file or directory/)
    assert.equal(readFileSync(binPath(blockedHome), 'utf8'), 'keep me')
  } finally {
    await new Promise(resolve => server.close(resolve))
    rmSync(root, { recursive: true, force: true })
  }
})
