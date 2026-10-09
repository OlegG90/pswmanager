import { api, type Entry, type Listing, type MergePreview } from './api'
import { busyButton, button, el } from './dom'
import { ask, dialog } from './modal'

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

/** `"Mail"`, or `3 entries`. */
const describe = (entries: Entry[]) => (entries.length === 1 ? `"${titleOf(entries[0])}"` : `${entries.length} entries`)

/** Asks to merge, showing what the kept entry gets (`preview`). */
function confirmMerge(message: string, preview: MergePreview): Promise<boolean> {
  const line = (name: string, value: string, hint = '') =>
    el('li', {}, el('span', { className: 'name' }, name), el('span', { className: 'value' }, value), hint ? el('span', { className: 'hint' }, hint) : '')
  const lines = [
    ...preview.fields.map((f) => line(f.name, f.value ?? 'hidden', f.fills ? 'was empty' : 'new field')),
    ...(preview.tags.length ? [line('Tags', preview.tags.join(', '))] : []),
    ...preview.files.map((name) => line('File', name)),
    ...(preview.icon ? [line('Icon', 'from another entry')] : []),
  ]
  const body = lines.length ? el('ul', { className: 'merge-preview' }, ...lines) : el('p', { className: 'muted' }, 'Nothing it does not have already.')
  return dialog(message, false, (answer) => ({ body: [body], buttons: [button('Merge', 'Merge', () => answer(true), 'danger')] }), 'Cancel', 'merge')
}

/** One site's entries, the most recently changed first. */
interface Site {
  name: string
  entries: Entry[]
}

/**
 * One site's entries. One is kept (the most recently changed, at first); the
 * others are ticked. Merge puts what the ticked ones have into the kept one
 * and moves them to the recycle bin; Move to the recycle bin moves the ticked
 * ones only, the kept one too when it is ticked.
 */
function siteSection(site: Site, options: SimilarOptions, redraw: () => Promise<void>, leave: () => void) {
  let keep = site.entries[0].id
  const rows = site.entries.map((entry) => {
    const tick = el('input', { type: 'checkbox', checked: entry.id !== keep, title: 'Merge or delete this one' })
    const keepRadio = el('input', { type: 'radio', name: `keep-${site.name}`, checked: entry.id === keep, title: 'Keep this one' })
    const row = el('label', { className: 'similar' },
      tick,
      options.icon(entry.id),
      el('span', { className: 'text' },
        el('span', {}, titleOf(entry)),
        el('span', { className: 'hint' }, [entry.username, entry.url].filter(Boolean).join(' · '))),
      el('span', { className: 'keep' }, keepRadio, 'Keep'))
    keepRadio.addEventListener('change', () => {
      // The one kept before is ticked in its place.
      rows.find((r) => r.entry.id === keep)!.tick.checked = true
      keep = entry.id
      tick.checked = false
    })
    return { entry, tick, row }
  })

  const keptEntry = () => rows.find((r) => r.entry.id === keep)!.entry
  /** The ticked entries, but for the kept one unless `withKept`; none is a mistake. */
  const ticked = (withKept: boolean) => {
    const chosen = rows.filter((r) => r.tick.checked && (withKept || r.entry.id !== keep)).map((r) => r.entry)
    if (!chosen.length) options.fail('Tick the entries to merge or delete')
    return chosen
  }
  async function merge() {
    const others = ticked(false)
    if (!others.length) return
    const into = titleOf(keptEntry())
    const ids = others.map((e) => e.id)
    const preview = await api.mergePreview(keep, ids)
    if (!await confirmMerge(`Merge ${describe(others)} into "${into}"? They go to the recycle bin; "${into}" gets:`, preview)) return
    options.changed(await api.mergeEntries(keep, ids), `Merged into "${into}"`)
    await redraw()
  }
  async function remove() {
    const chosen = ticked(true)
    if (!chosen.length) return
    const stays = chosen.some((e) => e.id === keep) ? '' : ` "${titleOf(keptEntry())}" stays as it is.`
    if (!await ask(`Move ${describe(chosen)} to the recycle bin?${stays}`, 'Move to the recycle bin')) return
    options.changed(await api.deleteEntries(chosen.map((e) => e.id)), `Moved ${describe(chosen)} to the recycle bin`)
    await redraw()
  }
  return el('section', {},
    el('h3', {}, site.name),
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
    const sites: Site[] = (await api.similarEntries())
      .filter((s) => !left.has(s.site))
      .map((s) => ({ name: s.site, entries: s.ids.map(options.entry).filter((e): e is Entry => e?.kind === 'entry') }))
      .filter((s) => s.entries.length > 1)
    const leave = (name: string) => () => {
      left.add(name)
      draw().catch((e) => options.fail(String(e)))
    }
    container.replaceChildren(
      el('header', {}, el('h1', {}, 'Similar entries'), button('Done', 'Back (Esc)', options.done)),
      el('p', { className: 'muted intro' }, 'Entries whose URLs are for the same site. Merging keeps one entry with everything the others have; they go to the recycle bin.'),
      ...(sites.length
        ? sites.map((s) => siteSection(s, options, draw, leave(s.name)))
        : [el('p', { className: 'muted' }, 'No two entries are for the same site.')]),
    )
  }
  await draw()
}
