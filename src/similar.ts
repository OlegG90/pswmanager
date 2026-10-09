import { api, type Entry, type Listing } from './api'
import { busyButton, button, el } from './dom'
import { ask } from './modal'

export interface SimilarOptions {
  done: () => void
  /** The entry as the list has it. */
  entry: (id: string) => Entry | undefined
  /** Draws an entry's icon. */
  icon: (id: string) => HTMLElement
  /** Shows the listing a merge or a delete left, saying what was done. */
  changed: (listing: Listing, message: string) => void
  fail: (message: string) => void
}

const titleOf = (entry: Entry) => entry.title || '(no title)'

/**
 * One site's entries. One is kept (the most recently changed, at first); the
 * others are ticked. Merge puts what the ticked ones have into the kept one
 * and moves them to the recycle bin; Move to the recycle bin moves them only.
 */
function site(name: string, entries: Entry[], options: SimilarOptions, redraw: () => Promise<void>, leave: () => void) {
  let keep = entries[0].id
  const rows = entries.map((entry) => {
    const tick = el('input', { type: 'checkbox', checked: entry.id !== keep, title: 'Merge or delete this one' })
    const kept = el('input', { type: 'radio', name: `keep-${name}`, checked: entry.id === keep, title: 'Keep this one' })
    const row = el('label', { className: 'similar' },
      tick,
      options.icon(entry.id),
      el('span', { className: 'text' },
        el('span', {}, titleOf(entry)),
        el('span', { className: 'hint' }, [entry.username, entry.url].filter(Boolean).join(' · '))),
      el('span', { className: 'keep' }, kept, 'Keep'))
    return { entry, tick, kept, row }
  })
  const show = () => rows.forEach((r) => (r.tick.style.visibility = r.entry.id === keep ? 'hidden' : ''))
  for (const r of rows) {
    r.kept.addEventListener('change', () => {
      // The one kept before is ticked in its place.
      rows.find((o) => o.entry.id === keep)!.tick.checked = true
      keep = r.entry.id
      r.tick.checked = false
      show()
    })
  }
  show()

  const ticked = () => rows.filter((r) => r.entry.id !== keep && r.tick.checked).map((r) => r.entry)
  const pick = () => {
    const chosen = ticked()
    if (!chosen.length) options.fail('Tick the entries to merge or delete')
    return chosen
  }
  const kept = () => rows.find((r) => r.entry.id === keep)!.entry
  async function merge() {
    const others = pick()
    if (!others.length) return
    const into = titleOf(kept())
    const what = others.length === 1 ? `"${titleOf(others[0])}"` : `${others.length} entries`
    if (!await ask(`Merge ${what} into "${into}"? What they have that "${into}" does not is added to it, and they go to the recycle bin.`, 'Merge')) return
    options.changed(await api.mergeEntries(keep, others.map((e) => e.id)), `Merged into "${into}"`)
    await redraw()
  }
  async function remove() {
    const others = pick()
    if (!others.length) return
    const what = others.length === 1 ? `"${titleOf(others[0])}"` : `${others.length} entries`
    if (!await ask(`Move ${what} to the recycle bin? "${titleOf(kept())}" stays as it is.`, 'Move to the recycle bin')) return
    options.changed(await api.deleteEntries(others.map((e) => e.id)), `Moved ${others.length === 1 ? 'it' : what} to the recycle bin`)
    await redraw()
  }
  return el('section', {},
    el('h3', {}, name),
    ...rows.map((r) => r.row),
    el('div', { className: 'similar-actions' },
      busyButton('Merge', 'Add what the ticked entries have to the kept one, and move them to the recycle bin', merge, options.fail, 'primary'),
      busyButton('Move to the recycle bin', 'Move the ticked entries to the recycle bin', remove, options.fail, 'danger'),
      button('Leave as is', 'Leave these entries as they are', leave)))
}

/** Fills `container` with the entries that are for the same site. */
export async function renderSimilar(container: HTMLElement, options: SimilarOptions) {
  /** Sites left as they are, while the report is open. */
  const left = new Set<string>()
  async function draw() {
    const sites = (await api.similarEntries())
      .filter((s) => !left.has(s.site))
      .map((s) => ({ site: s.site, entries: s.ids.map(options.entry).filter((e): e is Entry => e?.kind === 'entry') }))
      .filter((s) => s.entries.length > 1)
    const leave = (name: string) => () => {
      left.add(name)
      draw().catch((e) => options.fail(String(e)))
    }
    container.replaceChildren(
      el('header', {}, el('h1', {}, 'Similar entries'), button('Done', 'Back (Esc)', options.done)),
      el('p', { className: 'muted intro' }, 'Entries whose URLs are for the same site. Merging keeps one entry with everything the others have; they go to the recycle bin.'),
      ...(sites.length
        ? sites.map((s) => site(s.site, s.entries, options, draw, leave(s.site)))
        : [el('p', { className: 'muted' }, 'No two entries are for the same site.')]),
    )
  }
  await draw()
}
