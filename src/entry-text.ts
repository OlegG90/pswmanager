/** How tags and group paths are written in one line of text. */

const GROUP_SEPARATOR = ' / '

export const formatGroup = (group: string[]) => group.join(GROUP_SEPARATOR)

/** `Work / Mail` → `['Work', 'Mail']`; blank parts are dropped. */
export const parseGroup = (text: string) =>
  text
    .split(GROUP_SEPARATOR.trim())
    .map((part) => part.trim())
    .filter(Boolean)

export const formatTags = (tags: string[]) => tags.join(', ')

/** KeePass separates tags with commas or semicolons. */
export const parseTags = (text: string) => [
  ...new Set(
    text
      .split(/[,;]/)
      .map((tag) => tag.trim())
      .filter(Boolean),
  ),
]
