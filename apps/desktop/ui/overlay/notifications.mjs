// Server-owned inbox data. The desktop never derives read, claim or expiry rules.
const INTEGER_ID = /^[1-9][0-9]{0,18}$/
const SAME_APP_PATH = /^\/(?!\/)[^\\\u0000-\u001f]*$/

export function notificationSnapshot(value) {
  if (!value || !Array.isArray(value.items) || !value.attention || !Number.isSafeInteger(value.attention.unread) || value.attention.unread < 0) return null
  const items = []
  for (const item of value.items.slice(0, 10)) {
    if (!item || typeof item.id !== 'string' || !INTEGER_ID.test(item.id)
      || typeof item.kind !== 'string' || item.kind.length > 80
      || BigInt(item.id) > 9223372036854775807n
      || typeof item.message !== 'string' || item.message.length > 1000
      || typeof item.href !== 'string' || !SAME_APP_PATH.test(item.href)
      || typeof item.at !== 'string' || !Number.isFinite(Date.parse(item.at))
      || typeof item.read !== 'boolean' || typeof item.claimable !== 'boolean') continue
    if (item.read && !item.claimable) continue
    items.push({
      id: item.id, kind: item.kind, href: item.href, at: item.at,
      sentence: typeof item.sentence === 'string' && item.sentence.length <= 1200 ? item.sentence : item.message,
      read: item.read, claimable: item.claimable,
    })
  }
  return { attention: { unread: value.attention.unread }, items }
}

export function bellCount(count) {
  return count > 9 ? '9+' : count > 0 ? String(count) : ''
}

export function notificationIcon(kind) {
  if (kind === 'achievement') return './assets/achievement-crest.svg'
  if (kind.startsWith('title')) return './assets/achievement-tome.svg'
  if (kind === 'pve_won' || kind === 'won') return './assets/pvp-win-skull.png'
  if (kind === 'pve_lost' || kind === 'lost') return './assets/pvp-loss-tomb.png'
  if (kind.startsWith('friend')) return './assets/add-friend.png'
  if (kind.startsWith('sponsor') || kind === 'daily_reward' || kind === 'promotion') return './assets/gold.png'
  return './assets/duel-challenge.png'
}

// The overlay moves with the hero; this local position stays on the sprite's upper edge.
export function bellPosition(rect) {
  const size = 30
  const x = rect.spriteX + 128 + size <= rect.width ? rect.spriteX + 106 : rect.spriteX - 30
  return {
    left: Math.max(0, Math.min(rect.width - size, x)),
    top: Math.max(0, Math.min(rect.height - size, rect.spriteY + 2)),
  }
}

// Unread rows only: a reward already read but still to claim stays in the list without ringing the bell.
export function unreadCount(snapshot) {
  return snapshot ? snapshot.items.filter(item => !item.read).length : 0
}

// What the inbox looks like once one row is read, before the server's refreshed snapshot arrives.
export function markedRead(snapshot, id) {
  const items = snapshot.items
    .map(item => item.id === id ? { ...item, read: true } : item)
    .filter(item => !item.read || item.claimable)
  const wasUnread = snapshot.items.some(item => item.id === id && !item.read)
  return { attention: { unread: Math.max(0, snapshot.attention.unread - (wasUnread ? 1 : 0)) }, items }
}

// The bell only shows while something is unread, and never while the hero is away or being carried.
export function bellVisible({ count, away, carried }) {
  return !away && !carried && count > 0
}
