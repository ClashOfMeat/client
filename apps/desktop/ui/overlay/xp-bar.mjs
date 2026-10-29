// The hero card's XP bar, drawn like the web app's daily reward meter (guildbyte-app
// components/daily-gauge): the level sits above the bar and the count is centred inside it.

/** Same as the web app's compactTokens: floored, so 15.49M never reads as 15.5M early. */
export function compactAmount(value) {
  if (value >= 1_000_000) return `${Math.floor(value / 100_000) / 10}M`
  if (value >= 1_000) return `${Math.floor(value / 1_000)}K`
  return String(Math.max(0, Math.floor(value)))
}

// state is 'progress', 'max' or 'unavailable'; fill is a percentage of the bar.
export function xpBar(character, xp) {
  const level = `Lv ${character?.level ?? 1}`
  const next = character?.nextLevelAt
  const floor = Number.isSafeInteger(character?.levelFloor) ? character.levelFloor : 0
  const span = next - floor
  const into = xp - floor
  if (Number.isSafeInteger(xp) && Number.isSafeInteger(next) && span > 0) {
    const shown = Math.max(0, Math.min(into, span))
    return { state: 'progress', level, count: `${compactAmount(shown)}/${compactAmount(span)}`, fill: shown / span * 100 }
  }
  if (Number.isSafeInteger(xp) && (next === null || span <= 0)) {
    return { state: 'max', level: `${level} · Max level`, count: `${compactAmount(xp)} XP`, fill: 100 }
  }
  return { state: 'unavailable', level, count: 'XP unavailable', fill: 0 }
}
