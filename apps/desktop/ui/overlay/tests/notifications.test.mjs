import test from 'node:test'
import assert from 'node:assert/strict'
import { notificationSnapshot, bellCount, notificationIcon, bellPosition } from '../notifications.mjs'

const item = (id, extra = {}) => ({
  id: String(id), kind: 'duel_invite', message: `Event ${id}`, sentence: `Event ${id} — claim 5 gold`,
  href: '/duels/received', at: '2026-11-03T12:00:00Z', read: false, claimable: false, ...extra,
})

test('inbox keeps the bounded unread list and rewards still claimable after reading', () => {
  const items = [item(20), item(19, { read: true, claimable: true }), item(18, { read: true }), ...Array.from({ length: 12 }, (_, n) => item(17 - n))]
  const snapshot = notificationSnapshot({ attention: { unread: 13 }, items })
  assert.deepEqual(snapshot.items.map(row => row.id), ['20', '19', '17', '16', '15', '14', '13', '12', '11'])
  assert.equal(snapshot.items[1].sentence, 'Event 19 — claim 5 gold')
  assert.equal(bellCount(snapshot.attention.unread), '9+')
  assert.equal(bellCount(0), '')
})

test('older app data and unsafe paths cannot create navigable rows', () => {
  assert.equal(notificationSnapshot(undefined), null)
  assert.equal(notificationSnapshot({ attention: {}, items: [] }), null)
  const snapshot = notificationSnapshot({ attention: { unread: 2 }, items: [
    item(1, { sentence: undefined }), item(2, { href: '//evil.example' }),
    item(3, { href: '/ok\\bad' }), item('bad'), item(4, { href: '/safe?bid=123#reward' }),
  ] })
  assert.deepEqual(snapshot.items.map(row => row.id), ['1', '4'])
  assert.equal(snapshot.items[0].sentence, 'Event 1')
})

test('web icon mapping uses the same local art', () => {
  assert.equal(notificationIcon('achievement'), './assets/achievement-crest.svg')
  assert.equal(notificationIcon('title_unlocked'), './assets/achievement-tome.svg')
  assert.equal(notificationIcon('pve_won'), './assets/pvp-win-skull.png')
  assert.equal(notificationIcon('friend_request'), './assets/add-friend.png')
  assert.equal(notificationIcon('sponsor_bid'), './assets/gold.png')
})

test('bell follows the hero and stays inside the overlay at both screen edges', () => {
  for (const spriteX of [0, 116, 232]) {
    const at = bellPosition({ width: 360, height: 380, spriteX, spriteY: 252 })
    assert.ok(at.left >= 0 && at.left + 30 <= 360)
    assert.ok(at.top >= 0 && at.top + 30 <= 380)
  }
  assert.notEqual(bellPosition({ width: 360, height: 380, spriteX: 0, spriteY: 252 }).left,
    bellPosition({ width: 360, height: 380, spriteX: 116, spriteY: 252 }).left)
})
