import { api, type Entry, type Listing } from './api'
import { busyButton, button, el } from './dom'
import { describeEntries, titleOf } from './entry-text'
import { leaveSite, mergedCard, NO_SIMILAR, NONE_TICKED, similarHint, SIMILAR_INTRO, siteChoice, similarSites, type Site } from './similar-parts'

export interface SimilarOptions {
  done: () => void
  /** The entry as the list has it. */
  entry: (id: string) => Entry | undefined
  /** Draws an entry's icon. */
  icon: (id: string) => HTMLElement
  /** Shows the listing a merge left, saying what was done. */
  changed: (listing: Listing, message: string) => void
  fail: (message: string) => void
}

/** What a site's section needs from the report it is in. */
interface Report {
  options: SimilarOptions
  /** Shows the report again from the backend, after a merge. */
  redraw: () => Promise<void>
  /** Shows `nodes` in the report's place; returns what puts the report back as it was. */
  replace: (nodes: Node[]) => () => void
}

/**
 * One site's entries. Proceed shows the kept one as the merge would leave it,
 * with Merge (what the ticked ones have goes into it, and they go to the
 * recycle bin) and Cancel (back to the sites, ticks as they were).
 */
function siteSection(site: Site, report: Report, leave: (section: HTMLElement) => void) {
  const { options } = report
  const choice = siteChoice(site, (entry, tick, keep) =>
    el('label', { className: 'similar' },
      tick,
      options.icon(entry.id),
      el('span', { className: 'text' }, el('span', {}, titleOf(entry)), el('span', { className: 'hint' }, similarHint(entry))),
      el('span', { className: 'keep' }, keep, 'Keep')))
  async function proceed() {
    const others = choice.toMerge()
    if (!others.length) return options.fail(NONE_TICKED)
    const kept = choice.kept()
    const into = `"${titleOf(kept)}"`
    const ids = others.map((e) => e.id)
    const preview = await api.mergePreview(kept.id, ids)
    const back = report.replace([
      el('header', {}, el('h1', {}, `Merge into ${into}`), button('Cancel', 'Back to the similar entries', () => back())),
      el('p', { className: 'muted intro' }, `${into} as the merge would leave it. ${describeEntries(others)} ${others.length === 1 ? 'goes' : 'go'} to the recycle bin.`),
      mergedCard(preview, options.icon(kept.id)),
      el('div', { className: 'similar-actions' },
        busyButton('Merge', 'Merge them into it', async () => {
          options.changed(await api.mergeEntries(kept.id, ids), `Merged into ${into}`)
          await report.redraw()
        }, options.fail, 'primary'),
        button('Cancel', 'Back to the similar entries', () => back())),
    ])
  }
  const section: HTMLElement = el('section', {},
    el('h3', {}, site.name),
    ...choice.rows,
    el('div', { className: 'similar-actions' },
      busyButton('Proceed', 'See the kept entry as the merge would leave it', proceed, options.fail, 'primary'),
      button('Leave as is', 'Leave these entries as they are', () => leave(section))))
  return section
}

/** Fills `container` with the entries that are for the same site. */
export async function renderSimilar(container: HTMLElement, options: SimilarOptions) {
  /** Sites left as they are, while the report is open. */
  const left = new Set<string>()
  const replace = (nodes: Node[]) => {
    const report = [...container.childNodes]
    container.replaceChildren(...nodes)
    container.querySelector('button')?.focus()
    return () => container.replaceChildren(...report)
  }
  async function draw() {
    const sites = similarSites(await api.similarEntries(), options.entry, left)
    const none = () => el('p', { className: 'muted' }, NO_SIMILAR)
    const leave = (name: string) => (section: HTMLElement) => {
      left.add(name)
      leaveSite(container, section, none)
    }
    const report: Report = { options, redraw: draw, replace }
    container.replaceChildren(
      el('header', {}, el('h1', {}, 'Similar entries'), button('Done', 'Back (Esc)', options.done)),
      el('p', { className: 'muted intro' }, SIMILAR_INTRO),
      ...(sites.length ? sites.map((s) => siteSection(s, report, leave(s.name))) : [none()]),
    )
  }
  await draw()
}
