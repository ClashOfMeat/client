import test from 'node:test'
import assert from 'node:assert/strict'
import { heroActivity } from '../activity.mjs'

const session = state => ({ provider: 'codex', session_id: 's1', state, last_at: 0 })
const random = (...values) => {
  let index = 0
  return () => values[index++] ?? 0
}

test('idle, Done talk, sit, and sleep use the activity clock', () => {
  const hero = heroActivity(0, { random: random(0, 0, 0) })
  assert.equal(hero.snapshot(19999, { width: 100 }).state, 'idle')
  assert.equal(hero.snapshot(20000, { width: 100 }).state, 'sit')
  assert.equal(hero.snapshot(90000, { width: 100 }).state, 'sleep')
  hero.update({ busy: true, sessions: [session('Busy')] }, 90001)
  assert.equal(hero.snapshot(90001, { width: 100 }).state, 'idle')
  assert.equal(hero.snapshot(200000, { width: 100 }).state, 'idle', 'busy sessions never sleep')
  hero.update({ busy: false, sessions: [session('Done')] }, 200000)
  assert.equal(hero.snapshot(200000, { width: 100 }).state, 'talk')
  assert.equal(hero.snapshot(201499, { width: 100 }).state, 'talk')
  const idle = hero.snapshot(201500, { width: 100 })
  assert.equal(idle.state, 'idle')
  assert.equal(idle.elapsed, 0)
  assert.equal(idle.x, 0)
  assert.equal(hero.snapshot(220000, { width: 100 }).state, 'sit')
  assert.equal(hero.snapshot(290000, { width: 100 }).state, 'sleep')
  hero.preview('wave', 290001)
  assert.equal(hero.snapshot(390000, { width: 100 }).state, 'wave')
  hero.update({ busy: false, sessions: [session('Idle')] }, 390001)
  assert.equal(hero.snapshot(390001, { width: 100 }).state, 'idle')
})

test('working schedules a short stroll and stands between strolls', () => {
  const hero = heroActivity(0, { random: random(0, 0.5, 0, 0.9, 0) })
  hero.update({ busy: true, sessions: [session('Busy')] }, 0)
  assert.deepEqual(hero.snapshot(0, { width: 300 }), { state: 'idle', elapsed: 0, x: 0, facing: 'right', until: 12000 })
  assert.deepEqual(hero.snapshot(11999, { width: 300 }), { state: 'idle', elapsed: 11999, x: 0, facing: 'right', until: 1 })
  assert.deepEqual(hero.snapshot(12000, { width: 300 }), { state: 'walk', elapsed: 0, x: 0, facing: 'right', until: Infinity })
  assert.equal(hero.snapshot(12500, { width: 300 }).x, 24)
  const arrived = hero.snapshot(13250, { width: 300 })
  assert.equal(arrived.state, 'idle')
  assert.equal(arrived.x, 60)
  assert.equal(arrived.until, 12000)
  assert(arrived.x >= 0 && arrived.x <= 300)
  assert.equal(hero.snapshot(120000, { width: 300 }).state, 'idle', 'working never sits or sleeps')
  const late = heroActivity(0, { random: random(0, 0.5, 0, 0.9, 0) })
  late.update({ busy: true }, 0)
  assert.deepEqual(late.snapshot(14000, { width: 300 }), { state: 'idle', elapsed: 750, x: 60, facing: 'right', until: 11250 })
})

test('a scheduled turn flips facing without moving', () => {
  const hero = heroActivity(0, { random: random(0, 0, 0) })
  hero.update({ busy: true }, 0)
  const turned = hero.snapshot(12000, { width: 300 })
  assert.equal(turned.state, 'idle')
  assert.equal(turned.x, 0)
  assert.equal(turned.facing, 'left')
  assert.equal(turned.until, 12000)
})

test('a stroll uses the other side near an edge and clamps its target', () => {
  const hero = heroActivity(0, { random: random(0, 0.5, 0.999, 0.9, 0) })
  hero.snapshot(0, { width: 100 })
  hero.drag(90, 0)
  hero.update({ busy: true }, 0)
  assert.equal(hero.snapshot(12000, { width: 100 }).facing, 'left')
  assert.equal(hero.snapshot(14000, { width: 100 }).x, 0)
  assert.equal(hero.snapshot(14000, { width: 100 }).state, 'idle')
})

test('a stroll faces its direction during and after walking', () => {
  const right = heroActivity(0, { random: random(0, 0.5, 0, 0, 0) })
  right.update({ busy: true }, 0)
  const rightWalking = right.snapshot(12500, { width: 100 })
  assert.deepEqual([rightWalking.state, rightWalking.x, rightWalking.facing], ['walk', 24, 'right'])
  const rightArrived = right.snapshot(13250, { width: 100 })
  assert.deepEqual([rightArrived.state, rightArrived.x, rightArrived.facing], ['idle', 60, 'right'])

  const left = heroActivity(0, { random: random(0, 0.5, 0, 0.9, 0) })
  left.snapshot(0, { width: 100 })
  left.drag(90, 0)
  left.update({ busy: true }, 0)
  const leftWalking = left.snapshot(12500, { width: 100 })
  assert.deepEqual([leftWalking.state, leftWalking.x, leftWalking.facing], ['walk', 66, 'left'])
  const leftArrived = left.snapshot(13250, { width: 100 })
  assert.deepEqual([leftArrived.state, leftArrived.x, leftArrived.facing], ['idle', 30, 'left'])
})

test('until counts down to a stroll and still postpones its start', () => {
  const hero = heroActivity(0, { random: random(0.5, 0.5, 0, 0.9) })
  hero.update({ busy: true }, 0)
  assert.equal(hero.snapshot(5000, { width: 300 }).until, 21000)
  assert.equal(hero.snapshot(26000, { width: 300, still: true }).until, Infinity)
  assert.equal(hero.snapshot(27000, { width: 300, still: true }).state, 'idle')
  assert.deepEqual(hero.snapshot(28000, { width: 300 }), { state: 'walk', elapsed: 0, x: 0, facing: 'right', until: Infinity })
  assert.equal(hero.snapshot(28500, { width: 300 }).x, 24)
})

test('a postponed stroll waits without requesting frames and starts when still clears', () => {
  const hero = heroActivity(0, { random: random(0, 0.5, 0, 0.9) })
  hero.update({ busy: true }, 0)
  assert.equal(hero.snapshot(12000, { width: 300, still: true }).until, Infinity)
  assert.equal(hero.snapshot(20000, { width: 300, still: true }).until, Infinity)
  assert.equal(hero.snapshot(20000, { width: 300 }).state, 'walk')
})

test('emote plays once, survives update, and restarts idle time at its end', () => {
  const hero = heroActivity(0, { random: random(0) })
  hero.emote('wave', 1000, 1150)
  hero.update({ busy: true }, 1500)
  assert.equal(hero.snapshot(2149, { width: 100 }).state, 'wave')
  assert.equal(hero.snapshot(2150, { width: 100 }).state, 'idle')
  hero.update({ busy: false }, 3000)
  const resting = heroActivity(0)
  resting.emote('wave', 90000, 1150)
  assert.equal(resting.snapshot(91149, { width: 100 }).state, 'wave')
  assert.equal(resting.snapshot(91150, { width: 100 }).state, 'idle')
  assert.equal(resting.snapshot(111149, { width: 100 }).state, 'idle')
  assert.equal(resting.snapshot(111150, { width: 100 }).state, 'sit')
})

test('drag during a stroll stops it at the dragged position', () => {
  const hero = heroActivity(0, { random: random(0, 0.5, 0, 0.9, 0) })
  hero.update({ busy: true }, 0)
  hero.snapshot(12000, { width: 200 })
  assert.equal(hero.snapshot(12500, { width: 200 }).x, 24)
  hero.drag(80, 12500)
  assert.deepEqual(hero.snapshot(12500, { width: 200 }), { state: 'idle', elapsed: 0, x: 80, facing: 'right', until: 12000 })
  assert.equal(hero.snapshot(20000, { width: 200 }).x, 80)
})

test('emote during a stroll holds the current position for the whole emote', () => {
  const hero = heroActivity(0, { random: random(0, 0.5, 0, 0.9, 0) })
  hero.update({ busy: true }, 0)
  hero.snapshot(12000, { width: 200 })
  const x = hero.snapshot(12500, { width: 200 }).x
  hero.emote('wave', 12500, 1000)
  assert.deepEqual([12500, 13000, 13499].map(at => hero.snapshot(at, { width: 200 }).x), [x, x, x])
  assert.equal(hero.snapshot(13500, { width: 200 }).x, x)
})

test('pause freezes elapsed, idle thresholds, stroll timing, and position', () => {
  const hero = heroActivity(0, { random: random(0, 0.5, 0, 0.9) })
  hero.update({ busy: true, sessions: [session('Busy')] }, 0)
  const before = hero.snapshot(1000, { width: 100 })
  assert.deepEqual(hero.snapshot(5000, { width: 100, paused: true }), before)
  assert.deepEqual(hero.snapshot(100000, { width: 100, paused: true }), before)
  assert.equal(hero.snapshot(101000, { width: 100 }).until, 10000)
  assert.equal(hero.snapshot(111000, { width: 100 }).state, 'walk')
  const walking = hero.snapshot(111500, { width: 100, paused: true })
  assert.equal(walking.x, 0)
  assert.deepEqual(hero.snapshot(112000, { width: 100, paused: true }), walking)
  assert.equal(hero.snapshot(112500, { width: 100 }).x, 24)
  hero.update({ busy: false, sessions: [session('Done')] }, 112500)
  assert.equal(hero.snapshot(113000, { width: 100, paused: true }).state, 'walk')
  assert.equal(hero.snapshot(300000, { width: 100, paused: true }).state, 'walk')
  assert.equal(hero.snapshot(300500, { width: 100 }).state, 'walk')
  assert.equal(hero.snapshot(301000, { width: 100 }).state, 'talk')
  assert.equal(hero.snapshot(302000, { width: 100 }).state, 'idle')
})

test('an initial Done report does not invent a completed turn', () => {
  const hero = heroActivity(0)
  hero.update({ busy: false, sessions: [session('Done')] }, 0)
  assert.equal(hero.snapshot(0, { width: 10 }).state, 'idle')
  hero.update({ busy: true, sessions: [session('Busy')] }, 1)
  assert.equal(hero.snapshot(100, { width: 10, canMirror: false }).x, 0)
  assert.equal(hero.snapshot(100, { width: 0 }).x, 0)
})
