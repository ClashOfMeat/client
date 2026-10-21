// The server re-sends a visit on every heartbeat until it is acknowledged, so each one plays once.
export function visitTracker(limit = 50) {
  const seen = []
  return {
    // True the first time this visit ID is offered.
    first(id) {
      if (seen.includes(id)) return false
      seen.push(id)
      if (seen.length > limit) seen.shift()
      return true
    },
  }
}

// Left edge of the "from <name>" label: centred over the visitor and kept inside the window,
// or null while the visitor is outside it.
export function visitLabelLeft(visitorX, labelWidth, windowWidth, sprite = 128) {
  const center = visitorX + sprite / 2
  if (center < 0 || center > windowWidth) return null
  return Math.round(Math.max(2, Math.min(windowWidth - labelWidth - 2, center - labelWidth / 2)))
}
