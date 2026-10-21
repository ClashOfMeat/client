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

// A failed daemon request rejects with the server's message ("Gab's app isn't running.").
export function errorText(error) {
  const text = typeof error === 'string' ? error : error?.message
  return typeof text === 'string' && text.trim() ? text.trim().slice(0, 160) : 'Something went wrong. Try again.'
}
