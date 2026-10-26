import test from 'node:test'
import assert from 'node:assert/strict'
import { outgoingVisits, scopedOutgoingVisits } from '../outgoing.mjs'

const A = '11111111-1111-4111-8111-111111111111'
const B = '22222222-2222-4222-8222-222222222222'
const at = ms => new Date(ms).toISOString()
const storage = () => {
  const entries = new Map()
  return {
    entries,
    getItem: key => entries.get(key) ?? null,
    setItem: (key, value) => entries.set(key, value),
    removeItem: key => entries.delete(key),
  }
}

test('successful send hides until the matching visit is played', () => {
  const visits = outgoingVisits()
  assert.equal(visits.observe({ id: A, status: 'pending', expiresAt: at(120_000) }, 0), true)
  assert.equal(visits.away(1_000), true)
  assert.equal(visits.observe(null, 2_000), false) // Stale heartbeat absence proves nothing.
  assert.equal(visits.observe({ id: A, status: 'delivered', expiresAt: at(120_000) }, 3_000), false)
  assert.equal(visits.observe({ id: A, status: 'played', expiresAt: at(120_000) }, 11_000), true)
  assert.equal(visits.away(11_000), false)
  assert.equal(visits.observe({ id: A, status: 'pending', expiresAt: at(120_000) }, 12_000), false)
})

test('two sends are tracked by ID; old completion cannot restore a newer away hero', () => {
  const visits = outgoingVisits()
  visits.observe({ id: A, expiresAt: at(120_000) }, 0)
  visits.observe({ id: B, status: 'pending', expiresAt: at(140_000) }, 20_000)
  assert.deepEqual(visits.ids(20_000), [A, B])
  visits.observe({ id: A, status: 'played', expiresAt: at(120_000) }, 21_000)
  assert.deepEqual(visits.ids(21_000), [B])
  assert.equal(visits.away(21_000), true)
  visits.observe({ id: B, status: 'failed', expiresAt: at(140_000) }, 22_000)
  assert.equal(visits.away(22_000), false)
})

test('restart snapshot restores away state; offline expiry restores the hero after two minutes', () => {
  const visits = outgoingVisits()
  visits.observe({ id: A, status: 'delivered', expiresAt: at(120_000) }, 60_000)
  const restarted = outgoingVisits()
  for (const saved of visits.snapshot(60_000)) restarted.observe(saved, 60_001)
  assert.equal(restarted.away(119_999), true)
  assert.equal(restarted.expire(120_000), true)
  assert.equal(restarted.away(120_000), false)
  assert.equal(restarted.observe({ id: A, status: 'pending', expiresAt: at(120_000) }, 120_001), false)
})

test('a completed visit stays completed after restart when a stale pending heartbeat arrives', () => {
  const visits = outgoingVisits()
  visits.observe({ id: A, status: 'played', expiresAt: at(120_000) }, 20_000)
  const restarted = outgoingVisits()
  for (const saved of visits.snapshot(20_000)) restarted.observe(saved, 20_001)
  assert.equal(restarted.observe({ id: A, status: 'pending', expiresAt: at(120_000) }, 20_002), false)
  assert.equal(restarted.away(20_002), false)
})

test('invalid server records never hide the hero', () => {
  const visits = outgoingVisits()
  for (const value of [null, {}, { id: 'bad', status: 'pending', expiresAt: at(120_000) },
    { id: A, status: 'unknown', expiresAt: at(120_000) }, { id: A, status: 'pending', expiresAt: 'bad' }]) {
    assert.equal(visits.observe(value, 0), false)
  }
  assert.equal(visits.away(0), false)
})

test('switching paired accounts keeps each account’s away state separate', () => {
  const saved = storage()
  const visits = scopedOutgoingVisits(saved)
  visits.identify('alice', 0)
  visits.sent({ id: A, status: 'pending', expiresAt: at(120_000) }, 0)
  assert.equal(visits.away(1_000), true)
  assert.ok(saved.entries.has('clashofmeat.outgoingVisits.v2.alice'))

  visits.identify('bob', 2_000)
  visits.heartbeat(null, 2_000, true)
  assert.equal(visits.away(2_000), false)
  assert.deepEqual(visits.ids(2_000), [])
  assert.equal(saved.entries.has('clashofmeat.outgoingVisits.v2.bob'), false)

  visits.identify('alice', 3_000)
  assert.deepEqual(visits.ids(3_000), [A])
  visits.status({ id: A, status: 'played', expiresAt: at(120_000) }, 4_000)
  assert.equal(visits.away(4_000), false)
  visits.identify('bob', 5_000)
  assert.equal(visits.away(5_000), false)
})

test('late null heartbeat cannot undo a local send; terminal status can', () => {
  const visits = scopedOutgoingVisits(storage())
  visits.identify('alice', 0)
  visits.sent({ id: A, status: 'pending', expiresAt: at(120_000) }, 1_000)
  visits.heartbeat(null, 2_000, true)
  assert.equal(visits.away(2_000), true)
  visits.status({ id: A, status: 'played', expiresAt: at(120_000) }, 3_000)
  assert.equal(visits.away(3_000), false)
  visits.heartbeat({ id: A, status: 'pending', expiresAt: at(120_000) }, 4_000, true)
  assert.equal(visits.away(4_000), false)
})

test('authoritative absence clears restored IDs while preserving a new local send', () => {
  const saved = storage()
  const previous = scopedOutgoingVisits(saved)
  previous.identify('alice', 0)
  previous.sent({ id: A, status: 'pending', expiresAt: at(120_000) }, 1_000)
  const visits = scopedOutgoingVisits(saved)
  visits.identify('alice', 2_000)
  visits.sent({ id: B, status: 'pending', expiresAt: at(140_000) }, 3_000)
  visits.heartbeat(null, 4_000, true)
  assert.deepEqual(visits.ids(4_000), [B])
  visits.status({ id: B, status: 'played', expiresAt: at(140_000) }, 5_000)
  assert.equal(visits.away(5_000), false)
})

test('restart restores an owned visit, then authoritative absence clears it', () => {
  const saved = storage()
  const before = scopedOutgoingVisits(saved)
  before.identify('alice', 0)
  before.sent({ id: A, status: 'pending', expiresAt: at(120_000) }, 1_000)

  const restarted = scopedOutgoingVisits(saved)
  restarted.identify('alice', 2_000)
  assert.equal(restarted.away(2_000), true)
  restarted.heartbeat(null, 3_000, false) // Cached snapshots cannot settle an outbound visit.
  assert.equal(restarted.away(3_000), true)
  restarted.heartbeat(null, 4_000, true)
  assert.equal(restarted.away(4_000), false)
  restarted.heartbeat({ id: A, status: 'pending', expiresAt: at(120_000) }, 5_000, true)
  assert.equal(restarted.away(5_000), false)
})
