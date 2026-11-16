import assert from 'node:assert/strict'
import test from 'node:test'
import { heroSettings } from '../hero.mjs'

test('hero stands still by default and walks only when walking was chosen', () => {
  assert.deepEqual(heroSettings(undefined), { show: true, screen: 'main', still: true, signs: true })
  assert.deepEqual(heroSettings({}), { show: true, screen: 'main', still: true, signs: true })
  assert.equal(heroSettings({ show: true, screen: 'main' }).still, true)
  assert.equal(heroSettings({ still: true }).still, true)
  assert.equal(heroSettings({ still: false }).still, false)
})

test('hero settings keep the stored screen and visibility', () => {
  assert.deepEqual(heroSettings({ show: false, screen: 'cursor', still: false, signs: false }), { show: false, screen: 'cursor', still: false, signs: false })
  assert.equal(heroSettings({ screen: 'display:1' }).screen, 'display:1')
})

test("friends' signs are allowed unless turned off", () => {
  assert.equal(heroSettings({}).signs, true)
  assert.equal(heroSettings({ signs: false }).signs, false)
})
