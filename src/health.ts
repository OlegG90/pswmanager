import { api, type Finding } from './api'
import { el } from './dom'

export interface HealthOptions {
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
  // The count is in red for reused and weak passwords, in amber for old ones, while there are any.
  const groups: { title: string; label: string; findings: Finding[]; severity: 'crit' | 'warn' }[] = [
    { title: 'Reused', label: 'Reused passwords', findings: health.reused, severity: 'crit' },
    { title: 'Weak', label: 'Weak passwords', findings: health.weak, severity: 'crit' },
    { title: 'Old', label: 'Unchanged for over a year', findings: health.old, severity: 'warn' },
  ]
  const stat = ({ label, findings, severity }: (typeof groups)[number]) =>
    el('div', { className: findings.length ? `stat ${severity}` : 'stat' }, el('span', { className: 'count' }, String(findings.length)), el('span', { className: 'hint' }, label))
  const shown = groups.filter(({ findings }) => findings.length)
  container.replaceChildren(
    el('header', {}, el('h1', {}, 'Password health')),
    el('p', { className: 'muted intro' }, 'Checked on this PC only. Nothing about your entries leaves the device.'),
    el('div', { className: 'stats' }, ...groups.map(stat)),
    ...(shown.length
      ? shown.map(({ title, findings }) => el('section', {}, el('h3', {}, title), el('div', { className: 'card' }, ...findings.map((f) => item(f, options)))))
      : [el('p', { className: 'muted' }, 'No reused, weak or old passwords.')]),
  )
}
