The desktop hero has three manual emotes: Flex, Kiss and Angry. The Kiss and
Angry buttons use `emote-kiss.png` and `emote-angry.png`. Flex uses the player's
current crest from `leagues/`.

The seven PNGs in `leagues/` are exact copies of the web app's leaderboard
crests. Each transparent `flex-fx/<league>.png` stacks twelve 180×200 frames;
`hero/flex.mjs` defines their playback timing and placement above the hero.
`angry-mark.png` is the shared large Angry effect. Grab has no button or effect
asset because its motion is authored into every hero's animation frames.

`pixel-arrow.svg` and `pixel-chevron.svg` copy the web app's pixel back arrow
(`PixelBackIcon`, mirrored for forward buttons) and its select chevron, at the
app's colours.
