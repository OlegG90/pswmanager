import { api, type Finding } from './api'
import { button, el } from './dom'

export interface HealthOptions {
  done: () => void
  /** Opens the entry's editor to change its password. */
  fix: (id: string) => void
  /** Draws an entry's icon. */
  icon: (id: string) => HTMLElement
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
  const groups: [string, string, Finding[]][] = [
    ['Reused', 'Reused passwords', health.reused],
    ['Weak', 'Weak passwords', health.weak],
    ['Old', 'Unchanged for over a year', health.old],
  ]
  const stat = ([, label, findings]: (typeof groups)[number]) =>
    el('div', { className: 'stat' }, el('span', { className: 'count' }, String(findings.length)), el('span', { className: 'hint' }, label))
  const shown = groups.filter(([, , findings]) => findings.length)
  container.replaceChildren(
    el('header', {}, el('h1', {}, 'Password health'), button('Done', 'Back (Esc)', options.done)),
    el('p', { className: 'muted intro' }, 'Checked on this PC only. Nothing about your entries leaves the device.'),
    el('div', { className: 'stats' }, ...groups.map(stat)),
    ...(shown.length
      ? shown.map(([title, , findings]) => el('section', {}, el('h3', {}, title), ...findings.map((f) => item(f, options))))
      : [el('p', { className: 'muted' }, 'No reused, weak or old passwords.')]),
  )
}
