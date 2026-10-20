import test from 'node:test'
import assert from 'node:assert/strict'
import { companionFrame, frameDelay, frameImage } from '../frames.mjs'

const animation = { frames: ['zero', 'one', 'two'], mirroredFrames: { 1: 'one-left' }, clips: {
  idle: { frames: [0], durations: [200] },
  walk: { frames: [0, 1, 2], durations: [100, 200, 100] },
  sleep: { frames: [0, 1, 2], durations: [100, 200, 100] },
  kiss: { frames: [0, 1, 2], durations: [100, 200, 100] },
} }

test('clip playback loops or holds the final kiss and sleep frame', () => {
  assert.equal(companionFrame(animation, 'walk', 0), 0)
  assert.equal(companionFrame(animation, 'walk', 100), 1)
  assert.equal(companionFrame(animation, 'walk', 300), 2)
  assert.equal(companionFrame(animation, 'walk', 400), 0)
  assert.equal(companionFrame(animation, 'unknown', 100), 0)
  for (const state of ['sleep', 'kiss']) {
    assert.equal(companionFrame(animation, state, 0), 0)
    assert.equal(companionFrame(animation, state, 300), 2)
    assert.equal(companionFrame(animation, state, 400), 2)
    assert.equal(companionFrame(animation, state, 100000), 2)
  }
  const carrying = { clips: { walk: { frames: Array.from({ length: 12 }, (_, i) => 16 + i), durations: Array(12).fill(100) } } }
  assert.equal(companionFrame(carrying, 'walk', 1100), 27)
  assert.equal(companionFrame(carrying, 'walk', 1200), 16)
})

test('frameDelay reaches each exact boundary and stops on a held frame', () => {
  assert.equal(frameDelay(animation, 'walk', 0), 100)
  assert.equal(frameDelay(animation, 'walk', 99), 1)
  assert.equal(frameDelay(animation, 'walk', 100), 200)
  assert.equal(frameDelay(animation, 'walk', 299), 1)
  assert.equal(frameDelay(animation, 'walk', 300), 100)
  assert.equal(frameDelay(animation, 'walk', 400), 100)
  assert.equal(frameDelay(animation, 'kiss', 299), 1)
  assert.equal(frameDelay(animation, 'kiss', 300), Infinity)
  assert.equal(frameDelay(animation, 'sleep', 100000), Infinity)
  assert.equal(frameDelay(animation, 'idle', 0), Infinity)
  assert.equal(frameDelay({ clips: { sleep: { frames: [0, 1, 1], durations: [100, 200, 100] } } }, 'sleep', 100), Infinity)
  assert.equal(frameDelay(null, 'walk', 0), Infinity)
})

test('left facing selects authored mirror or requests a horizontal flip', () => {
  assert.deepEqual(frameImage(animation, 1, 'left'), { png: 'one-left', flip: false })
  assert.deepEqual(frameImage(animation, 2, 'left'), { png: 'two', flip: true })
  assert.deepEqual(frameImage(animation, 2, 'right'), { png: 'two', flip: false })
})
