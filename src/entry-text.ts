/** How tags and group paths are written in one line of text. */

const GROUP_SEPARATOR = ' / '

export const formatGroup = (group: string[]) => group.join(GROUP_SEPARATOR)

/** `Work / Mail` → `['Work', 'Mail']`; blank parts are dropped. Only " / "
 *  separates, so a name like `TCP/IP` stays whole. */
export const parseGroup = (text: string) =>
  text
    .split(GROUP_SEPARATOR)
    .map((part) => part.trim())
    .filter(Boolean)

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
