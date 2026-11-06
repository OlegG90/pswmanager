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

/** Fills `container` with the password health report. */
export async function renderHealth(container: HTMLElement, options: HealthOptions) {
  const health = await api.passwordHealth()
  let only = options.only ?? null

  // A figure with findings shows its list alone (#352), and clicked again
  // every list; one with none is no control. Its count is in its colour
  // while there are any.
  const stat = ({ group, label, severity }: (typeof HEALTH_GROUPS)[number]) => {
    const count = el('span', { className: 'count' }, String(health[group].length))
    const hint = el('span', { className: 'hint' }, label)
    if (!health[group].length) return el('div', { className: 'stat' }, count, hint)
    const alone = only === group
    const b = el('button', {
      type: 'button', className: `stat ${severity}`, ariaPressed: String(alone),
      title: alone ? 'Show every list' : `Show only: ${label.toLowerCase()}`,
      onclick: () => {
        only = alone ? null : group
        draw()
        container.querySelector<HTMLElement>(`button.stat[data-group='${group}']`)?.focus()
      },
    }, count, hint)
    b.dataset.group = group
    return b
  }

  const draw = () => {
    const shown = shownGroups(health, only)
    container.replaceChildren(
      el('header', {}, el('h1', {}, 'Password health')),
      el('p', { className: 'muted intro' }, 'Checked on this PC only. Nothing about your entries leaves the device.'),
      el('div', { className: 'stats' }, ...HEALTH_GROUPS.map(stat)),
      ...(shown.length
        ? shown.map(({ title, group }) => el('section', {}, el('h3', {}, title), el('div', { className: 'card' }, ...health[group].map((f: Finding) => item(f, options)))))
        : [el('p', { className: 'muted' }, 'No reused, weak or old passwords.')]),
    )
  }
  draw()
}
