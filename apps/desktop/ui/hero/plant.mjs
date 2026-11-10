// Review me: the visitor plants a sign beside it. A light emote made only from the contract's stand and talk
// poses, squashed and lifted on the canvas, so every hero plays it without new art. Times are from the end of
// the walk in (HERO_CONTRACT.visit.walkInMs):
//   0–150 crouch · 150–350 hop up 16 px · 350–450 drop · 450–570 stomp (the sign lands) · 570–750 recover ·
//   750–1350 "Review me!" talk beat, turned to the host.
export const SIGN_EMOTE = 'review_me'
export const PLANT_MS = 1350
// The sign starts falling here and lands 300 ms later, on the stomp.
export const SIGN_DROP_AT = 150
export const SIGN_LANDS_AT = 450
const TALK = [4, 5, 6, 5]

const lerp = (a, b, k) => a + (b - a) * k

// { frame, sx, sy, dy, toHost }: the pose index, the squash about the feet, the lift (negative is up), and
// whether the visitor has turned from its sign to the host.
export function plantPose(elapsed) {
  const t = Math.max(0, Math.min(PLANT_MS - 1, elapsed))
  if (t < 150) { const k = t / 150; return { frame: 0, sx: 1 + 0.06 * k, sy: 1 - 0.1 * k, dy: 0, toHost: false } }
  if (t < 350) { const k = 1 - (1 - (t - 150) / 200) ** 2; return { frame: 0, sx: lerp(1.06, 0.95, k), sy: lerp(0.9, 1.07, k), dy: -16 * k, toHost: false } }
  if (t < 450) { const k = ((t - 350) / 100) ** 2; return { frame: 0, sx: lerp(0.95, 1, k), sy: lerp(1.07, 1, k), dy: -16 * (1 - k), toHost: false } }
  if (t < 570) return { frame: 0, sx: 1.1, sy: 0.88, dy: 0, toHost: false }
  if (t < 750) { const k = (t - 570) / 180; return { frame: 0, sx: lerp(1.1, 1, k), sy: lerp(0.88, 1, k), dy: 0, toHost: false } }
  return { frame: TALK[Math.floor((t - 750) / 150) % TALK.length], sx: 1, sy: 1, dy: 0, toHost: true }
}

// Where the sign's post stands, in screen px: beside the visitor on the side away from the host, kept on the
// screen. visitorCenter is the visitor's middle on screen, outward +1 when the host is on its left.
export function signPostX(visitorCenter, outward, area, gap = 110, half = 78) {
  return Math.round(Math.max(area.x + half, Math.min(area.x + area.width - half, visitorCenter + outward * gap)))
}
