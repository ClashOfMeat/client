export const COMPANION_STATES = ['idle', 'walk', 'sit', 'talk', 'kiss', 'wave', 'laugh', 'angry', 'victory', 'sleep']

const BASE64 = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/
const PNG_SIGNATURE = '\x89PNG\r\n\x1a\n'
const UUID = /^[a-f\d]{8}-[a-f\d]{4}-[a-f\d]{4}-[a-f\d]{4}-[a-f\d]{12}$/i
const count = value => Number.isSafeInteger(value) && value >= 0

export function validPng(png, limit = 64 * 1024) {
  if (typeof png !== 'string' || png.length > limit || !BASE64.test(png)) return false
  try { return atob(png.slice(0, 12)).slice(0, 8) === PNG_SIGNATURE }
  catch { return false }
}

export function validateAnimation(value) {
  if (!value || value.version !== 1 || !Array.isArray(value.frames) || ![20, 24, 28, 32].includes(value.frames.length) ||
      value.frames.some(png => !validPng(png)) || value.frames.reduce((n, png) => n + png.length, 0) > 1024 * 1024 || !value.clips) return null
  const clips = {}
  for (const state of COMPANION_STATES) {
    const clip = value.clips[state]
    if (!clip || !Array.isArray(clip.frames) || !Array.isArray(clip.durations) || !clip.frames.length || clip.frames.length > 24 ||
        clip.frames.length !== clip.durations.length || clip.frames.some(i => !Number.isInteger(i) || i < 0 || i >= value.frames.length) ||
        clip.durations.some(ms => !Number.isFinite(ms) || ms < 60 || ms > 10000)) return null
    clips[state] = { frames: clip.frames, durations: clip.durations }
  }
  let mirroredFrames
  if (value.mirroredFrames !== undefined) {
    if (!value.mirroredFrames || typeof value.mirroredFrames !== 'object' || Array.isArray(value.mirroredFrames)) return null
    const entries = Object.entries(value.mirroredFrames)
    if (entries.length > value.frames.length || entries.some(([frame, png]) => !/^\d+$/.test(frame) || String(Number(frame)) !== frame || Number(frame) >= value.frames.length || !validPng(png)) ||
        entries.reduce((n, [, png]) => n + png.length, 0) > 1024 * 1024) return null
    mirroredFrames = Object.fromEntries(entries)
  }
  return { version: 1, frames: value.frames, clips, ...(mirroredFrames ? { mirroredFrames } : {}) }
}

export function validateCharacter(character) {
  if (!character || typeof character !== 'object' || !UUID.test(character.id) || !validPng(character.png, 2 * 1024 * 1024)) return null
  const animation = validateAnimation(character.animation)
  if (character.level !== undefined && (!Number.isInteger(character.level) || character.level < 1)) return null
  if (character.xp !== undefined && character.xp !== null && !count(character.xp)) return null
  if (character.levelFloor !== undefined && character.levelFloor !== null && !count(character.levelFloor)) return null
  if (character.nextLevelAt !== undefined && character.nextLevelAt !== null && !count(character.nextLevelAt)) return null
  if (character.canLevelUp !== undefined && typeof character.canLevelUp !== 'boolean') return null
  if (character.heroId !== undefined && (typeof character.heroId !== 'string' || !/^[a-z0-9_]{1,80}$/.test(character.heroId))) return null
  return { id: character.id, png: character.png,
    ...(character.xp !== undefined ? { xp: character.xp } : {}),
    ...(character.levelFloor !== undefined ? { levelFloor: character.levelFloor } : {}),
    ...(character.nextLevelAt !== undefined ? { nextLevelAt: character.nextLevelAt } : {}),
    ...(character.canLevelUp !== undefined ? { canLevelUp: character.canLevelUp } : {}),
    ...(character.level !== undefined ? { level: character.level } : {}),
    ...(character.heroId ? { heroId: character.heroId } : {}),
    ...(animation ? { animation } : {}) }
}

export function validateLevelUp(value) {
  if (!value || value.version !== 1 || typeof value.id !== 'string' || value.id.length > 80 ||
      typeof value.heroId !== 'string' || value.heroId.length > 80 ||
      ![value.fromLevel, value.toLevel].every(n => Number.isInteger(n) && n >= 1) ||
      !Array.isArray(value.frames) || value.frames.length !== 18 || value.frames.some(png => !validPng(png, value.renderScale === 1.5 ? 96 * 1024 : 64 * 1024)) ||
      value.frames.reduce((n, png) => n + png.length, 0) > 1024 * 1024 ||
      !Array.isArray(value.durations) || value.durations.length !== 18 ||
      value.durations.some(ms => !Number.isInteger(ms) || ms < 60 || ms > 1000) ||
      (value.renderScale !== undefined && ![1, 1.5].includes(value.renderScale)) ||
      value.durationMs !== value.durations.reduce((a, b) => a + b, 0) || value.durationMs > 10000) return null
  return { version: 1, id: value.id, heroId: value.heroId, fromLevel: value.fromLevel, toLevel: value.toLevel,
    frames: value.frames, durations: value.durations, durationMs: value.durationMs,
    ...(value.renderScale !== undefined ? { renderScale: value.renderScale } : {}) }
}

export function validateVisit(value, now = Date.now()) {
  if (!value || typeof value !== 'object' || !UUID.test(value.id) || typeof value.name !== 'string' || value.name.length > 80 ||
      !(value.guild === null || typeof value.guild === 'string' && value.guild.length <= 80) ||
      typeof value.expiresAt !== 'string') return null
  const expires = Date.parse(value.expiresAt)
  if (!Number.isFinite(expires) || expires <= now || expires > now + 300000) return null
  const character = validateCharacter(value.character)
  if (!character?.animation) return null
  const clean = text => text.replace(/[\x00-\x1f\x7f]/g, '')
  return { id: value.id, name: clean(value.name), guild: value.guild === null ? null : clean(value.guild), expiresAt: value.expiresAt, character }
}
