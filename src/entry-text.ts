/** How tags, dates and sizes are written in one line of text. */

export const formatTags = (tags: string[]) => tags.join(', ')

/** Line breaks an <input> cannot hold. */
export const singleLine = (text: string) => text.replace(/[\r\n]/g, '')

/** Line ends a <textarea> hands back. */
export const textareaLines = (text: string) => text.replace(/\r\n?/g, '\n')

/**
 * What to save for a value the editor showed as `original` and now holds as
 * `edited`: the original when the only difference is what showing it did
 * (dropped line breaks, trimmed spaces, tidied tags), so an untouched entry
 * saves unchanged. The backend's three-way merge relies on this: a value that
 * only looked edited would overwrite another device's change to it.
 */
export function keep<T>(original: T, edited: T, shown: (value: T) => T): T {
  return JSON.stringify(shown(original)) === JSON.stringify(edited) ? original : edited
}

/** KeePass separates tags with commas or semicolons. */
export const parseTags = (text: string) => [
  ...new Set(
    text
      .split(/[,;]/)
      .map((tag) => tag.trim())
      .filter(Boolean),
  ),
]

/** An expiry time as a date input shows it: the day on this PC, `2030-01-02`. */
export function dateOf(iso: string): string {
  const at = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`
}

/** A date input's day as an expiry time: its start on this PC. */
export function startOfDay(date: string): string {
  const [year, month, day] = date.split('-').map(Number)
  return new Date(year, month - 1, day).toISOString()
}

/** The day an expiry time falls on, as people read it. */
export const formatDate = (iso: string) => new Date(iso).toLocaleDateString(undefined, { dateStyle: 'medium' })

/** A moment as people read it, on this PC: `12 Sep 2026, 14:05`. */
export const formatDateTime = (iso: string) => new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })

/** A file size as people read it: `820 B`, `14 KB`, `2.4 MB`. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const [value, unit] = bytes < 1024 * 1024 ? [bytes / 1024, 'KB'] : [bytes / 1024 / 1024, 'MB']
  return `${value < 10 ? value.toFixed(1).replace(/\.0$/, '') : Math.round(value)} ${unit}`
}
