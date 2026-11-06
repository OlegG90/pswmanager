import { api, type Finding } from './api'
import { el } from './dom'
import { HEALTH_GROUPS, type HealthGroup, shownGroups } from './health-parts'

export interface HealthOptions {
  /** Opens the entry's editor to change its password. */
  fix: (id: string) => void
  /** Draws an entry's icon. */
  icon: (id: string) => HTMLElement
  /** The list shown alone at first, as when its figure is clicked. */
  only?: HealthGroup
}

function item(finding: Finding, options: HealthOptions): HTMLButtonElement {
  return el('button', { type: 'button', className: 'finding', title: 'Change the password', onclick: () => options.fix(finding.id) },
    options.icon(finding.id),
    el('span', { className: 'text' }, el('span', {}, finding.title), el('span', { className: 'hint' }, finding.detail)),
    el('span', { className: 'action' }, 'Change password'))
}

/** A figure: its count and label; while it has any, a button with `props`
 *  in its colour, else plain text. Home shows the same figures (#351). */
export function statFigure(count: number, label: string, severity: string, props: object): HTMLElement {
  const parts = [el('span', { className: 'count' }, String(count)), el('span', { className: 'hint' }, label)]
  return count
    ? el('button', { type: 'button', className: `stat ${severity}`, ...props }, ...parts)
    : el('div', { className: 'stat' }, ...parts)
}

/** Fills `container` with the password health report. */
export async function renderHealth(container: HTMLElement, options: HealthOptions) {
  const health = await api.passwordHealth()
  let only = options.only ?? null

  // A figure with findings shows its list alone (#352), and clicked again
  // every list.
  const stat = ({ group, label, severity }: (typeof HEALTH_GROUPS)[number]) => {
    const alone = only === group
    const figure = statFigure(health[group].length, label, severity, {
      ariaPressed: String(alone),
      title: alone ? 'Show every list' : `Show only: ${label.toLowerCase()}`,
      onclick: () => {
        only = alone ? null : group
        draw()
        container.querySelector<HTMLElement>(`button.stat[data-group='${group}']`)?.focus()
      },
    })
    figure.dataset.group = group
    return figure
  }

  const draw = () => {
    const shown = shownGroups(health, only)
    container.replaceChildren(
      el('header', {}, el('h1', {}, 'Password health')),
      el('p', { className: 'muted intro' }, 'Checked on this PC only. Nothing about your entries leaves the device.'),
      el('div', { className: 'stats' }, ...HEALTH_GROUPS.map(stat)),
      ...(shown.length
        ? shown.map(({ title, group }) => el('section', {}, el('h3', {}, title), el('div', { className: 'card' }, ...health[group].map((f) => item(f, options)))))
        : [el('p', { className: 'muted' }, 'No reused, weak or old passwords.')]),
    )
  }
  draw()
}
