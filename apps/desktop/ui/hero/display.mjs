// A display scale changes the visible hero inside its 128px animation cell.
// Keep the cell center fixed and the feet on the same floor line at every size.
export function safeDisplayScale(value) {
  return Number.isFinite(value) && value >= 0.5 && value <= 2 ? value : 1
}

export function heroDrawRect(x, y, scale, width = 128, height = 128) {
  const factor = safeDisplayScale(scale)
  const drawnWidth = Math.round(width * factor)
  const drawnHeight = Math.round(height * factor)
  return {
    x: x + Math.round((width - drawnWidth) / 2),
    y: y + height - drawnHeight,
    width: drawnWidth,
    height: drawnHeight,
  }
}
