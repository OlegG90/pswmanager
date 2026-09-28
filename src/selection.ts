/** Several entries chosen in the list, as Windows lists choose them. */
export interface Choice {
  /** In the list's order. */
  chosen: string[]
  /** Where a Shift+click range starts. */
  anchor: string
}

/**
 * What a click on `id` chooses in the list `ids`, given what is chosen now:
 * a plain click chooses it alone; Ctrl+click adds or takes it off (never
 * leaving nothing); Shift+click chooses the range from the anchor to it.
 */
export function clicked(ids: string[], now: Choice | null, id: string, keys: { ctrl: boolean; shift: boolean }): Choice {
  if (keys.shift && now) {
    const [from, to] = [ids.indexOf(now.anchor), ids.indexOf(id)]
    if (from >= 0 && to >= 0) {
      return { chosen: ids.slice(Math.min(from, to), Math.max(from, to) + 1), anchor: now.anchor }
    }
  }
  if (keys.ctrl && now) {
    const set = new Set(now.chosen)
    if (set.has(id) && set.size > 1) set.delete(id)
    else set.add(id)
    return { chosen: ids.filter((i) => set.has(i)), anchor: id }
  }
  return { chosen: [id], anchor: id }
}
