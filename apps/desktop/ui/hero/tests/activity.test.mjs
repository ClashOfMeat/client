import test from 'node:test'
import assert from 'node:assert/strict'
import { heroActivity } from '../activity.mjs'

const session = state => ({ provider: 'codex', session_id: 's1', state, last_at: 0 })

test('idle, Done talk, sit, and sleep use the activity clock', () => {
  const hero = heroActivity(0)
  assert.equal(hero.snapshot(19999, { width: 100 }).state, 'idle')
  assert.equal(hero.snapshot(20000, { width: 100 }).state, 'sit')
  assert.equal(hero.snapshot(90000, { width: 100 }).state, 'sleep')
  hero.update({ busy: true, sessions: [session('Busy')] }, 90001)
  assert.equal(hero.snapshot(90001, { width: 100 }).state, 'walk')
  assert.equal(hero.snapshot(200000, { width: 100 }).state, 'walk', 'busy sessions never sleep')
  hero.update({ busy: false, sessions: [session('Done')] }, 200000)
  assert.deepEqual(hero.snapshot(200000, { width: 100 }).state, 'talk')
  assert.equal(hero.snapshot(201499, { width: 100 }).state, 'talk')
  const idle = hero.snapshot(201500, { width: 100 })
  assert.equal(idle.state, 'idle')
  assert.equal(idle.elapsed, 0)
  assert(idle.x > 0 && idle.x < 100, 'the hero remains at its last patrol position')
  assert.equal(hero.snapshot(220000, { width: 100 }).state, 'sit')
  assert.equal(hero.snapshot(290000, { width: 100 }).state, 'sleep')
  hero.preview('wave', 290001)
  assert.equal(hero.snapshot(390000, { width: 100 }).state, 'wave')
  hero.update({ busy: false, sessions: [session('Idle')] }, 390001)
  assert.equal(hero.snapshot(390001, { width: 100 }).state, 'idle')
})

test('patrol reflects at both ends, remains bounded, and stops at its current position', () => {
  const hero = heroActivity(0)
  hero.update({ busy: true, sessions: [session('Busy')] }, 0)
  assert.deepEqual(hero.snapshot(1000, { width: 100 }), { state: 'walk', elapsed: 1000, x: 48, facing: 'left' })
  assert.deepEqual(hero.snapshot(2500, { width: 100 }), { state: 'walk', elapsed: 2500, x: 80, facing: 'right' })
  let left = false, right = false
  for (let at = 2550; at <= 20000; at += 50) {
    const pose = hero.snapshot(at, { width: 100 })
    assert(pose.x >= 0 && pose.x <= 100)
    left ||= pose.facing === 'left'
    right ||= pose.facing === 'right'
  }
  assert(left && right)
  const stopped = hero.snapshot(20000, { width: 100 })
  hero.update({ busy: false, sessions: [session('Done')] }, 20000)
  assert.equal(hero.snapshot(23000, { width: 100 }).x, stopped.x)
  assert.equal(hero.snapshot(110000, { width: 100 }).x, stopped.x)
})

test('drag clamps coordinates and patrol resumes from the dragged point', () => {
  const hero = heroActivity(0)
  hero.snapshot(0, { width: 100 })
  hero.drag(150, 0)
  assert.equal(hero.snapshot(0, { width: 100 }).x, 100)
  hero.update({ busy: true, sessions: [session('Busy')] }, 0)
  assert.deepEqual(hero.snapshot(1000, { width: 100 }), { state: 'walk', elapsed: 1000, x: 52, facing: 'right' })
  hero.drag(-10, 1000)
  assert.equal(hero.snapshot(1000, { width: 100 }).x, 0)
  assert.equal(hero.snapshot(1500, { width: 100 }).x, 24)
})

test('pause freezes elapsed, idle thresholds, and patrol position', () => {
  const hero = heroActivity(0)
  hero.update({ busy: true, sessions: [session('Busy')] }, 0)
  const before = hero.snapshot(1000, { width: 100 })
  assert.deepEqual(hero.snapshot(5000, { width: 100, paused: true }), before)
  assert.deepEqual(hero.snapshot(100000, { width: 100, paused: true }), before)
  assert.equal(hero.snapshot(101000, { width: 100 }).x, 96)
  hero.update({ busy: false, sessions: [session('Done')] }, 101000)
  assert.equal(hero.snapshot(102000, { width: 100, paused: true }).state, 'talk')
  assert.equal(hero.snapshot(200000, { width: 100, paused: true }).state, 'talk')
  assert.equal(hero.snapshot(201499, { width: 100 }).state, 'talk')
  assert.equal(hero.snapshot(201500, { width: 100 }).state, 'idle')
})

test('an initial Done report does not invent a completed turn', () => {
  const hero = heroActivity(0)
  hero.update({ busy: false, sessions: [session('Done')] }, 0)
  assert.equal(hero.snapshot(0, { width: 10 }).state, 'idle')
  hero.update({ busy: true, sessions: [session('Busy')] }, 1)
  assert.equal(hero.snapshot(100, { width: 10, canMirror: false }).x, 0)
  assert.equal(hero.snapshot(100, { width: 0 }).x, 0)
})
