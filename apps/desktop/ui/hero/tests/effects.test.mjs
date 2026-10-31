import test from 'node:test'
import assert from 'node:assert/strict'
import { xpGain, bubbleFrame, levelUpFrame, visitPose, visitPlace, visitDuration } from '../effects.mjs'

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
  // Enters from the right edge (offset 240), stands 84 px right of the host, faces it, leaves right.
  assert.deepEqual(visitPose(animation, 0, 84, 240), { frame: 0, state: 'walk', facing: 'left', offset: 240 })
  assert.equal(visitPose(animation, 750, 84, 240).offset, 162)
  assert.equal(visitPose(animation, 1500, 84, 240).frame, 0)
  assert.equal(visitPose(animation, 1600, 84, 240).frame, 1)
  assert.equal(visitPose(animation, 1900, 84, 240).frame, 2)
  assert.equal(visitPose(animation, 3000, 84, 240).facing, 'left')
  assert.equal(visitPose(animation, 6000, 84, 240).frame, 2)
  assert.equal(visitPose(animation, 6500, 84, 240).offset, 84)
  assert.equal(visitPose(animation, 7000, 84, 240).facing, 'right')
  assert.equal(visitPose(animation, 8000, 84, 240).offset, 240)
  // From the left it mirrors: walks right in, faces right towards the host, walks left out.
  assert.equal(visitPose(animation, 0, -84, -200).facing, 'right')
  assert.equal(visitPose(animation, 3000, -84, -200).facing, 'right')
  assert.equal(visitPose(animation, 7000, -84, -200).facing, 'left')
})

test('a visitor plays flex once between walking in and out, or a kiss without the clip', () => {
  const flexing = { clips: { ...animation.clips, flex: { frames: [5, 6, 7, 8], durations: [300, 125, 125, 3000] } } }
  assert.equal(visitDuration(flexing, 'flex'), 6550)
  assert.equal(visitDuration(animation, 'kiss'), 8000)
  assert.equal(visitPose(flexing, 1000, 84, 240, 'flex').state, 'walk')
  assert.deepEqual([1500, 1800, 1925, 2050, 5049].map(ms => visitPose(flexing, ms, 84, 240, 'flex').frame), [5, 6, 7, 8, 8])
  assert.equal(visitPose(flexing, 3000, 84, 240, 'flex').state, 'flex')
  assert.equal(visitPose(flexing, 5050, 84, 240, 'flex').state, 'walk')
  assert.equal(visitPose(flexing, 6550, 84, 240, 'flex').offset, 240)
  assert.equal(visitPose(animation, 3000, 84, 240, 'wave').state, 'kiss')
  assert.equal(visitPose(animation, 3000, 84, 240).state, 'kiss')
})

test('a visitor stands beside the host on the side of the window with room', () => {
  assert.deepEqual(visitPlace(56, 240), { stand: 56, edge: 184 })
  assert.deepEqual(visitPlace(20, 240), { stand: 84, edge: 220 })
  assert.deepEqual(visitPlace(112, 240), { stand: -84, edge: -240 })
})
