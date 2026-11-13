// The hero card's XP bar, drawn like the web app's daily reward meter (guildbyte-app
// components/daily-gauge): the level sits on the left of the bar and the count is centred inside it.

const whole = value => Math.max(0, Math.floor(value)).toLocaleString('en-US')

// state is 'progress', 'max' or 'unavailable'; fill is a percentage of the bar. The numbers are the
// web hero page's: total XP over the total the next level takes, and the bar fills by that share.
export function xpBar(character, xp) {
  const level = `Lv ${character?.level ?? 1}`
  const next = character?.nextLevelAt
  const floor = Number.isSafeInteger(character?.levelFloor) ? character.levelFloor : 0
  if (Number.isSafeInteger(xp) && Number.isSafeInteger(next) && next > floor) {
    const shown = Math.max(0, Math.min(xp, next))
    return { state: 'progress', level, count: `${whole(shown)} / ${whole(next)} XP`, fill: shown / next * 100 }
  }
  if (Number.isSafeInteger(xp) && (next === null || next <= floor)) {
    return { state: 'max', level, count: `${whole(xp)} XP · MAX`, fill: 100 }
  }
  return { state: 'unavailable', level, count: 'XP unavailable', fill: 0 }
}
