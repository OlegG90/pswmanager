import type { Module } from './api'

/**
 * Settings › Modules, as both apps behave (#412): which modules get a switch,
 * in which groups, what each row says, when a switch cannot be used, and the
 * setting a switch saves. Each app draws the rows its own way.
 *
 * - The import and export formats get no switch: turning one off only loses
 *   a feature.
 * - A folder or a platform document is not a module that can be turned off
 *   (`optional` false): it is how the app reaches a file at all.
 * - A store a database here syncs with stays on (`inUse`): its switch is
 *   disabled, saying why, until that database stops syncing there; the core
 *   refuses the setting too.
 * - The autofill categories only where the autofill router runs (the
 *   phone): on Windows a switch would change nothing while Auto-Type and the
 *   browser extension go on filling.
 */

/** A group of switches: its title, what it is about, and its modules. */
export interface ModuleGroup {
  title: string
  about: string
  modules: Module[]
}

/** The groups, in order; `phone` only where autofill runs. */
const GROUPS: { kind: Module['kind']; title: string; about: string; phone: boolean }[] = [
  {
    kind: 'stores',
    title: 'Stores',
    about: 'A cloud store turned off is not offered for syncing. A folder is always there.',
    phone: false,
  },
  {
    kind: 'fillers',
    title: 'Autofill categories',
    about: 'What PswManager fills in other apps and browsers. A card is filled only after the fingerprint or the screen lock.',
    phone: true,
  },
]

/** The groups of switches the app shows (`phone`: the Android app), empty ones left out. */
export function moduleGroups(modules: Module[], phone: boolean): ModuleGroup[] {
  return GROUPS.filter((g) => phone || !g.phone)
    .map(({ kind, title, about }) => ({ title, about, modules: modules.filter((m) => m.kind === kind && m.optional) }))
    .filter((g) => g.modules.length)
}

/** Whether a module's switch can be used: not for a store a database syncs with, while it is on. */
export const switchable = (m: Module) => !(m.inUse && m.enabled)

/** What a module's row says: its hint, and why a store in use cannot be turned off. */
export const moduleHint = (m: Module) =>
  switchable(m) ? m.hint : `${m.hint} A database here syncs with it: stop syncing there to turn it off.`

/** The `disabledModules` setting once `m` is turned `on` or off, from the keys turned off now. */
export function turned(disabled: string[], m: Module, on: boolean): string[] {
  const off = new Set(disabled)
  const key = `${m.kind}:${m.id}`
  if (on) off.delete(key)
  else off.add(key)
  return [...off].sort()
}
