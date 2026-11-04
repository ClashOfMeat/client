import test from 'node:test'
import assert from 'node:assert/strict'
import { decodePng } from '../png-image.mjs'

function fakeImage(decode = async () => {}) {
  return () => ({ src: '', decode })
}

test('a frame decodes into an image element from its base64 PNG', async () => {
  let decoded = false
  const image = await decodePng('iVBORw0KGgo=', fakeImage(async () => { decoded = true }))
  assert.equal(image.src, 'data:image/png;base64,iVBORw0KGgo=')
  assert.equal(decoded, true, 'the image is decoded before it is handed back')
})

test('a frame that cannot be decoded rejects, so the caller can retry it', async () => {
  await assert.rejects(decodePng('broken', fakeImage(async () => { throw new Error('EncodingError') })), /EncodingError/)
})
