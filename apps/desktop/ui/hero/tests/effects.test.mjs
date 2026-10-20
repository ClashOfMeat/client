import test from 'node:test'
import assert from 'node:assert/strict'
import { xpGain, bubbleFrame, levelUpFrame, visitPose } from '../effects.mjs'

const animation = { clips: { idle: { frames: [0], durations: [100] }, walk: { frames: [0, 1, 2], durations: [100, 200, 100] }, kiss: { frames: [0, 1, 2], durations: [100, 200, 100] } } }
const aura = { durations: [80, ...Array(16).fill(90), 120] }

test('XP bubble appears only for a positive difference on the same character', () => {
  assert.deepEqual(xpGain({ id: 'a', xp: 1_999_900 }, { id: 'a', xp: 2_000_000 }, 42), { amount: 100, startedAt: 42 })
  assert.deepEqual(xpGain({ id: 'a', level: 29, xp: 100 }, { id: 'a', level: 30, xp: 150 }, 43), { amount: 50, startedAt: 43 })
  assert.equal(xpGain({ id: 'a', xp: 100 }, { id: 'a', xp: 100 }, 0), null)
  assert.equal(xpGain({ id: 'a', xp: 100 }, { id: 'a', xp: 50 }, 0), null)
  assert.equal(xpGain({ id: 'a', xp: 100 }, { id: 'b', xp: 200 }, 0), null)
  assert.equal(xpGain({ id: 'a', xp: null }, { id: 'a', xp: 200 }, 0), null)
})

test('bubble floats and fades over 1800 ms', () => {
  const bubble = { amount: 100, startedAt: 1000 }
  assert.deepEqual(bubbleFrame(bubble, 1000), { visible: true, dy: 0, opacity: 1 })
  assert.equal(bubbleFrame(bubble, 1900).dy, -16)
  assert.equal(bubbleFrame(bubble, 2600).opacity, 200 / 350)
  assert.deepEqual(bubbleFrame(bubble, 2800), { visible: false, dy: -32, opacity: 0 })
})

test('aura and visitor keep old one-shot timing with pixel travel', () => {
  assert.equal(levelUpFrame(aura, 0), 0)
  assert.equal(levelUpFrame(aura, 80), 1)
  assert.equal(levelUpFrame(aura, 100000), 17)
  assert.deepEqual(visitPose(animation, 0, 144, 400), { frame: 0, state: 'walk', facing: 'right', offset: 336 })
  assert.equal(visitPose(animation, 750, 144, 400).offset, 240)
  assert.equal(visitPose(animation, 1500, 144, 400).frame, 0)
  assert.equal(visitPose(animation, 1600, 144, 400).frame, 1)
  assert.equal(visitPose(animation, 1900, 144, 400).frame, 2)
  assert.equal(visitPose(animation, 6000, 144, 400).frame, 2)
  assert.equal(visitPose(animation, 6500, 144, 400).offset, 144)
  assert.equal(visitPose(animation, 8000, 144, 400).offset, 336)
  assert.equal(visitPose(animation, 8000, 144, 400).facing, 'left')
  assert.equal(visitPose(animation, 0, 144, 200).offset, 200)
})
