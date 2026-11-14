import test from 'node:test'
import assert from 'node:assert/strict'
import { checkReviewLink, linkLabel } from '../review-link.mjs'

test('only https links on exactly github.com or gitlab.com pass', () => {
  for (const link of [
    'https://github.com/ClashOfMeat/app/pull/114',
    'https://gitlab.com/group/project/-/merge_requests/7#note_12',
    'https://github.com/org/repo/blob/main/app.ts?plain=1#L10',
    'https://GitHub.com/org/repo/issues/3',
  ]) assert.equal(checkReviewLink(link).ok, true, link)
  const why = link => checkReviewLink(link).why
  assert.equal(why(''), '')
  assert.equal(why('http://github.com/a/b'), 'Must be https://, not http://.')
  assert.equal(why('github.com/a/b'), 'Must start with https://')
  assert.equal(why('https://user@github.com/a'), 'No login or password in the link.')
  assert.equal(why('https://github.com:8443/a'), 'No port number.')
  assert.equal(why('https://gist.github.com/a'), 'Only github.com or gitlab.com links.')
  assert.equal(why('https://gitlab.example.com/a'), 'Only github.com or gitlab.com links.')
  assert.equal(why('https://github.com.evil.com/a'), 'Only github.com or gitlab.com links.')
  assert.equal(why('https://github.com/a b'), 'No spaces or backslashes.')
  assert.equal(why('https://github.com\\@evil.com'), 'No spaces or backslashes.')
  assert.equal(why(`https://github.com/${'a'.repeat(2048)}`), 'Link is too long (2048 max).')
})

test('the hover card shows the host, then the rest cut to fit', () => {
  assert.deepEqual(linkLabel('https://github.com/ClashOfMeat/app/pull/114'), { host: 'github.com', rest: '/ClashOfMeat/app/pull/114' })
  const long = linkLabel('https://github.com/ClashOfMeat/clashofmeat-app/pull/114/files?w=1#diff')
  assert.equal(long.host.length + long.rest.length, 44)
  assert(long.rest.endsWith('…'))
  assert.deepEqual(linkLabel('not a url'), { host: '', rest: '' })
})
