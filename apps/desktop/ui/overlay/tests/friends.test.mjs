import test from 'node:test'
import assert from 'node:assert/strict'
import { friendRows, friendPage, errorText, FRIENDS_PER_PAGE } from '../friends.mjs'

test('friend rows put running apps first and drop malformed entries', () => {
  assert.deepEqual(friendRows({ friends: [
    { handle: 'tom', name: 'Tom', online: false },
    { handle: 'gab', name: 'Gab', online: true, level: 3 },
    { name: 'no handle', online: true },
    { handle: 'lea', online: 'yes' },
    { handle: 'max', name: '', online: true },
  ] }), [
    { handle: 'gab', name: 'Gab', online: true },
    { handle: 'max', name: 'max', online: true },
    { handle: 'tom', name: 'Tom', online: false },
    { handle: 'lea', name: 'lea', online: false },
  ])
  assert.deepEqual(friendRows(null), [])
  assert.deepEqual(friendRows({ friends: 'x' }), [])
})

test('errors show the server message, or a fallback', () => {
  assert.equal(errorText("Gab's app isn't running."), "Gab's app isn't running.")
  assert.equal(errorText(new Error('daemon closed connection')), 'daemon closed connection')
  assert.equal(errorText(undefined), 'Something went wrong. Try again.')
  assert.equal(errorText('x'.repeat(300)).length, 160)
})

test('the friend list shows five per page, and an out-of-range page falls back to the nearest one', () => {
  const rows = Array.from({ length: 12 }, (_, i) => ({ handle: `f${i}` }))
  const handles = result => result.rows.map(row => row.handle)
  assert.equal(FRIENDS_PER_PAGE, 5)
  assert.deepEqual(handles(friendPage(rows, 0)), ['f0', 'f1', 'f2', 'f3', 'f4'])
  assert.deepEqual(friendPage(rows, 2), { rows: rows.slice(10), page: 2, pages: 3 })
  assert.equal(friendPage(rows, 9).page, 2)
  assert.equal(friendPage(rows, -1).page, 0)
  assert.deepEqual(friendPage([], 0), { rows: [], page: 0, pages: 1 })
  assert.deepEqual(friendPage(rows.slice(0, 5), 1), { rows: rows.slice(0, 5), page: 0, pages: 1 })
})
