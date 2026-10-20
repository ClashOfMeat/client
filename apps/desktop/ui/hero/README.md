# Hero animation modules

These browser ES modules also run in Node 22. They hold no timers or DOM state, and never read prompt text.

| Module | Job |
| --- | --- |
| `validate.mjs` | Check heartbeat PNGs, animation clips, characters, level-up effects, and visits. Levels and safe-integer XP have no game cap. |
| `activity.mjs` | Turn daemon `activity` reports into the hero pose and pixel position. `update` takes `{busy, sessions}`; `snapshot` takes the overlay's available travel width and current time. `drag` and `preview` change the pose without timers. |
| `frames.mjs` | Pick clip frames, time the next frame boundary, and choose an authored mirrored PNG or a canvas flip. |
| `effects.mjs` | Detect XP gains and compute bubble, level-up, and visitor poses. Visitor offsets and bubble `dy` are pixels. |

On each daemon activity event, call `hero.update(activity, now)`. On a heartbeat, validate the character, animation, level-up and visit payloads, then call `xpGain(previous, current, now)`. Render `hero.snapshot(now, { width: screenWidth - spriteWidth, paused, canMirror })`; call `companionFrame`, `frameImage`, and the active effect functions for the image. Schedule the next render for the earlier of the next movement step (at most about 12 frames per second), `frameDelay`, or an effect/state deadline. A held `sleep` or `kiss` frame returns `Infinity`, so stop the timer when nothing else is moving. Recompute on daemon events, drag, pause, and display changes.

Run `node --test apps/desktop/ui/hero/tests/` from the repository root.
