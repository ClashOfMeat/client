// The daemon's friend list, cleaned for the panel: apps running now first.
export function friendRows(value) {
  if (!Array.isArray(value?.friends)) return []
  const rows = value.friends
    .filter(friend => typeof friend?.handle === 'string' && friend.handle && friend.handle.length <= 80)
    .map(friend => ({
      handle: friend.handle,
      name: typeof friend.name === 'string' && friend.name && friend.name.length <= 80 ? friend.name : friend.handle,
      online: friend.online === true,
    }))
  return [...rows.filter(friend => friend.online), ...rows.filter(friend => !friend.online)]
}

// Five rows fit the bubble without scrolling. A page past the end (the list shrank) falls back to the last one.
export const FRIENDS_PER_PAGE = 5

export function friendPage(rows, page, size = FRIENDS_PER_PAGE) {
  const pages = Math.max(1, Math.ceil(rows.length / size))
  const current = Math.min(Math.max(0, Number.isInteger(page) ? page : 0), pages - 1)
  return { rows: rows.slice(current * size, (current + 1) * size), page: current, pages }
}

// A failed daemon request rejects with the server's message ("Gab's app isn't running.").
export function errorText(error) {
  const text = typeof error === 'string' ? error : error?.message
  return typeof text === 'string' && text.trim() ? text.trim().slice(0, 160) : 'Something went wrong. Try again.'
}
