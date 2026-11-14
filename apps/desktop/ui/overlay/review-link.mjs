// The link a Review me sign carries: https on exactly github.com or gitlab.com, with no login or port. Query
// and fragment are allowed. The server checks the same rule before it sends a sign, and the desktop checks it
// again before a planted sign opens (settings.rs review_link).
export const REVIEW_HOSTS = ['github.com', 'gitlab.com']

// { ok, why }: why is empty for an empty field, so the link step shows nothing until something is typed.
export function checkReviewLink(raw) {
  if (typeof raw !== 'string' || !raw) return { ok: false, why: '' }
  if (raw.length > 2048) return { ok: false, why: 'Link is too long (2048 max).' }
  if (/[\s\u0000-\u001f\u007f\\]/.test(raw)) return { ok: false, why: 'No spaces or backslashes.' }
  if (!/^https:\/\//i.test(raw)) return { ok: false, why: /^http:\/\//i.test(raw) ? 'Must be https://, not http://.' : 'Must start with https://' }
  const authority = raw.slice(8).split(/[/?#]/)[0]
  if (authority.includes('@')) return { ok: false, why: 'No login or password in the link.' }
  if (authority.includes(':')) return { ok: false, why: 'No port number.' }
  if (!REVIEW_HOSTS.includes(authority.toLowerCase())) return { ok: false, why: 'Only github.com or gitlab.com links.' }
  try { new URL(raw) } catch { return { ok: false, why: 'Not a valid link.' } }
  return { ok: true, why: '' }
}

// The link as a sign's hover card shows it: host, then path, query and fragment, cut to fit.
export function linkLabel(url, max = 44) {
  let parsed
  try { parsed = new URL(url) } catch { return { host: '', rest: '' } }
  const host = parsed.host.toLowerCase()
  let rest = parsed.pathname + parsed.search + parsed.hash
  if (host.length + rest.length > max) rest = rest.slice(0, Math.max(0, max - host.length - 1)) + '…'
  return { host, rest }
}
