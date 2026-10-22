// Run with the web app's tsx runtime from its root:
//   node_modules/.bin/tsx ../clashofmeat-client/scripts/check-hero-emote-payload.mjs . --all
// This checks the actual exported PNG payload, including authored mirrors.
import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { resolve, join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { COMPANION_STATES, LEAGUES, validateAnimation } from '../apps/desktop/ui/hero/validate.mjs'
import { FLEX_DURATION } from '../apps/desktop/ui/hero/flex.mjs'

const root = resolve(process.argv[2] ?? '.')
process.chdir(root)
const { LOOKS, appearanceForLook } = await import(pathToFileURL(join(root, 'lib/character-rig.ts')).href)
const { companionAnimation } = await import(pathToFileURL(join(root, 'lib/companion-animation.ts')).href)
const examples = new Set([
  'framework_familiar_lv5', // largest exported frame payload
  'complaint_countess_lv4', // vehicle
  'cash_cow_solopreneur_lv3',
  'brinebound_builder_lv1', // seated
  'hallucination_muse_lv5',
])
const selected = process.argv.includes('--all') ? LOOKS : LOOKS.filter(look => examples.has(look.id))
assert.equal(selected.length, process.argv.includes('--all') ? 105 : examples.size)

let largestFrames = 0, largestMirrors = 0
for (const look of selected) {
  const payload = await companionAnimation({ id: look.id, appearance: appearanceForLook(look), level: look.level })
  const accepted = validateAnimation(payload)
  assert(accepted, `desktop rejected ${look.id}`)
  for (const state of COMPANION_STATES) assert(accepted.clips[state], `${look.id} missing ${state}`)
  assert.deepEqual([...new Set(accepted.clips.grab.frames)].sort((a, b) => a - b), [0, 9, 10, 11, 15], `${look.id} grab poses`)
  assert(accepted.clips.flex.frames.includes(13), `${look.id} flex pose`)
  assert.equal(accepted.clips.flex.durations.reduce((sum, ms) => sum + ms, 0), FLEX_DURATION, `${look.id} flex timing`)
  assert.equal(Object.keys(accepted.mirroredFrames ?? {}).length, accepted.frames.length, `${look.id} authored mirror count`)
  largestFrames = Math.max(largestFrames, accepted.frames.reduce((sum, png) => sum + png.length, 0))
  largestMirrors = Math.max(largestMirrors, Object.values(accepted.mirroredFrames).reduce((sum, png) => sum + png.length, 0))
}
for (const league of LEAGUES) {
  assert(existsSync(new URL(`../apps/desktop/ui/assets/leagues/${league}.png`, import.meta.url)), `missing ${league} crest`)
  assert(existsSync(new URL(`../apps/desktop/ui/assets/flex-fx/${league}.png`, import.meta.url)), `missing ${league} flex effect`)
}
console.log(`${selected.length} exported hero payloads accepted; max frame bytes ${largestFrames}; max mirrored bytes ${largestMirrors}`)
