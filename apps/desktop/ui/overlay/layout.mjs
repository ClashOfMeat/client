const clamp = (n, min, max) => Math.min(max, Math.max(min, n))

// x is the sprite's left edge in patrol coordinates, not the window's edge.
export function windowRect(x, { workArea, sprite = 128, bubble = false } = {}) {
  const size = Math.max(1, sprite)
  const width = Math.min(workArea.width, bubble ? 240 : 160)
  const height = Math.min(workArea.height, bubble ? 310 : 160)
  const spriteLeft = workArea.x + clamp(x, 0, Math.max(0, workArea.width - size))
  const wanted = spriteLeft - (width - size) / 2
  const left = clamp(wanted, workArea.x, workArea.x + workArea.width - width)
  const top = workArea.y + workArea.height - height
  return {
    x: left, y: top, width, height,
    spriteX: spriteLeft - left,
    spriteY: height - size,
  }
}

export function heroBox({ workArea, sprite = 128, bubble = false } = {}) {
  return {
    patrolWidth: Math.max(0, workArea.width - sprite),
    ...windowRect(0, { workArea, sprite, bubble }),
  }
}

export function formatTokens(n) {
  if (!Number.isFinite(n) || n < 0) return ''
  if (n >= 1_000_000) return `${Number((n / 1_000_000).toFixed(1))}M`
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`
  return String(Math.round(n))
}
