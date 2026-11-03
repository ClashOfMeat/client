// From the game checkout: node --import remix/node-tsx ../clashofmeat-client/scripts/check-hero-emote-payload.mjs . --all
import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { resolve, join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { COMPANION_STATES, LEAGUES, validateAnimation } from '../apps/desktop/ui/hero/validate.mjs'
import { HERO_CONTRACT } from '../apps/desktop/ui/hero/hero-contract.mjs'
import { FLEX_DURATION } from '../apps/desktop/ui/hero/flex.mjs'

const root = resolve(process.argv[2] ?? '.')
process.chdir(root)
assert.deepEqual(JSON.parse(await readFile(join(root, 'packages/core/src/heroes/hero-contract.json'), 'utf8')), HERO_CONTRACT,
  'The game and desktop must share the same hero contract')
const { LOOKS, appearanceForLook } = await import(pathToFileURL(join(root, 'packages/core/src/heroes/character-rig.ts')).href)
const { companionAnimation } = await import(pathToFileURL(join(root, 'packages/core/src/heroes/companion-animation.ts')).href)
const selected = process.argv.includes('--all') ? LOOKS : LOOKS.filter(look => look.level === 5)
assert(selected.length > 0, 'No hero artwork was selected')
let largestFrames = 0, largestMirrors = 0
for (const look of selected) {
  const payload = await companionAnimation({ id: look.id, appearance: appearanceForLook(look), level: look.level })
  const accepted = validateAnimation(payload)
  assert(accepted, `desktop rejected ${look.id}`)
  for (const state of COMPANION_STATES) assert(accepted.clips[state], `${look.id} missing ${state}`)
  assert.deepEqual(accepted.clips, HERO_CONTRACT.clips, `${look.id} clip contract`)
  assert.equal(accepted.clips.flex.durations.reduce((sum, ms) => sum + ms, 0), FLEX_DURATION, `${look.id} Flex timing`)
  assert.equal(accepted.mirroredFrames.length, accepted.frames.length, `${look.id} mirror count`)
  largestFrames = Math.max(largestFrames, accepted.frames.reduce((sum, png) => sum + png.length, 0))
  largestMirrors = Math.max(largestMirrors, accepted.mirroredFrames.reduce((sum, png) => sum + png.length, 0))
}
for (const league of LEAGUES) {
  assert(existsSync(new URL(`../apps/desktop/ui/assets/leagues/${league}.png`, import.meta.url)), `missing ${league} crest`)
  assert(existsSync(new URL(`../apps/desktop/ui/assets/flex-fx/${league}.png`, import.meta.url)), `missing ${league} Flex effect`)
}
console.log(`${selected.length} exported hero payloads accepted; max frame bytes ${largestFrames}; max mirrored bytes ${largestMirrors}`)
