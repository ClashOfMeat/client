// Gravity for a dropped hero, in px/s². A drop from the top of a laptop screen lands in about 0.75 s.
export const GRAVITY = 3200
const BOUNCE = 0.22, MAX_BOUNCE_SPEED = 420

// Height above the ground `elapsed` ms after letting go at `from` px: one fall, one small bounce.
export function fallPose(from, elapsed) {
  const height = Math.max(0, from), t = Math.max(0, elapsed) / 1000
  if (height < 2) return { lift: 0, landed: true }
  const drop = Math.sqrt(2 * height / GRAVITY)
  if (t < drop) return { lift: height - GRAVITY * t * t / 2, landed: false }
  const speed = Math.min(MAX_BOUNCE_SPEED, BOUNCE * GRAVITY * drop), hop = 2 * speed / GRAVITY, since = t - drop
  if (since < hop) return { lift: speed * since - GRAVITY * since * since / 2, landed: false }
  return { lift: 0, landed: true }
}
