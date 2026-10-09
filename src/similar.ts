import { api, type Entry, type Listing, type MergePreview } from './api'
import { busyButton, button, el } from './dom'
import { describeEntries, titleOf } from './entry-text'
import { leaveSite, mergePreviewParts, NO_SIMILAR, NONE_TICKED, similarHint, SIMILAR_INTRO, siteChoice, similarSites, type Site } from './similar-parts'
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

/** Asks to merge, showing what the kept entry gets (`preview`). */
function confirmMerge(message: string, preview: MergePreview): Promise<boolean> {
  return dialog(message, false, (answer) => ({ body: mergePreviewParts(preview), buttons: [button('Merge', 'Merge', () => answer(true), 'danger')] }), 'Cancel', 'merge')
}

/**
 * One site's entries. Merge puts what the ticked ones have into the kept one
 * and moves them to the recycle bin; Move to the recycle bin moves the ticked
 * ones only, the kept one too when it is ticked.
 */
function siteSection(site: Site, options: SimilarOptions, redraw: () => Promise<void>, leave: (section: HTMLElement) => void) {
  const choice = siteChoice(site, (entry, tick, keep) =>
    el('label', { className: 'similar' },
      tick,
      options.icon(entry.id),
      el('span', { className: 'text' }, el('span', {}, titleOf(entry)), el('span', { className: 'hint' }, similarHint(entry))),
      el('span', { className: 'keep' }, keep, 'Keep')), (t) => `"${t}"`)
  async function merge() {
    const others = choice.toMerge()
    if (!others.length) return options.fail(NONE_TICKED)
    const kept = choice.kept()
    const into = titleOf(kept)
    const ids = others.map((e) => e.id)
    const preview = await api.mergePreview(kept.id, ids)
    if (!await confirmMerge(`Merge ${describeEntries(others)} into "${into}"? They go to the recycle bin; "${into}" gets:`, preview)) return
    options.changed(await api.mergeEntries(kept.id, ids), `Merged into "${into}"`)
    await redraw()
  }
  async function remove() {
    const chosen = choice.toDelete()
    if (!chosen.length) return options.fail(NONE_TICKED)
    if (!await ask(choice.deleteQuestion(chosen), 'Move to the recycle bin')) return
    options.changed(await api.deleteEntries(chosen.map((e) => e.id)), `Moved ${describeEntries(chosen)} to the recycle bin`)
    await redraw()
  }
  const section: HTMLElement = el('section', {},
    el('h3', {}, site.name),
    ...choice.rows,
    el('div', { className: 'similar-actions' },
      busyButton('Merge', 'Add what the ticked entries have to the kept one, and move them to the recycle bin', merge, options.fail, 'primary'),
      busyButton('Move to the recycle bin', 'Move the ticked entries to the recycle bin', remove, options.fail, 'danger'),
      button('Leave as is', 'Leave these entries as they are', () => leave(section))))
  return section
}

/** Fills `container` with the entries that are for the same site. */
export async function renderSimilar(container: HTMLElement, options: SimilarOptions) {
  /** Sites left as they are, while the report is open. */
  const left = new Set<string>()
  async function draw() {
    const sites = similarSites(await api.similarEntries(), options.entry, left)
    const none = () => el('p', { className: 'muted' }, NO_SIMILAR)
    const leave = (name: string) => (section: HTMLElement) => {
      left.add(name)
      leaveSite(container, section, none)
    }
    container.replaceChildren(
      el('header', {}, el('h1', {}, 'Similar entries'), button('Done', 'Back (Esc)', options.done)),
      el('p', { className: 'muted intro' }, SIMILAR_INTRO),
      ...(sites.length
        ? sites.map((s) => siteSection(s, options, draw, leave(s.name)))
        : [none()]),
    )
  }
  await draw()
}
