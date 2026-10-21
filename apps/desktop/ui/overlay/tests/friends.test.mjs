import test from 'node:test'
import assert from 'node:assert/strict'
import { friendRows, errorText } from '../friends.mjs'

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
