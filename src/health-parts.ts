import type { Finding } from './api'

/** A list of *Password health*: each entry is in the first that applies. */
export type HealthGroup = 'reused' | 'weak' | 'old'

/** The lists in their order, each with its figure's label; red for reused
 *  and weak passwords, amber for old ones. */
export const HEALTH_GROUPS: { group: HealthGroup; title: string; label: string; severity: 'crit' | 'warn' }[] = [
  { group: 'reused', title: 'Reused', label: 'Reused passwords', severity: 'crit' },
  { group: 'weak', title: 'Weak', label: 'Weak passwords', severity: 'crit' },
  { group: 'old', title: 'Old', label: 'Unchanged for over a year', severity: 'warn' },
]

/** The lists shown: `only` alone while it has findings (its figure clicked,
 *  #352), else every list that has any. */
export function shownGroups(health: Record<HealthGroup, Finding[]>, only: HealthGroup | null) {
  const listed = HEALTH_GROUPS.filter(({ group }) => health[group].length)
  const alone = listed.filter(({ group }) => group === only)
  return alone.length ? alone : listed
}
