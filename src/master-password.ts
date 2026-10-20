/** What a new master password must have (#224), for both apps. A database
 *  opened elsewhere may already have a password that breaks them: the rules
 *  apply only when a new one is chosen. */
import { el } from './dom'

const RULES: [string, (password: string) => boolean][] = [
  ['At least 6 characters', (p) => [...p].length >= 6],
  ['Upper- and lower-case letters', (p) => /\p{Lu}/u.test(p) && /\p{Ll}/u.test(p)],
  ['A character that is neither a letter nor a digit', (p) => /[^\p{L}\p{N}]/u.test(p)],
]

/** The rules `password` does not meet yet. */
export const unmetRules = (password: string) => RULES.filter(([, met]) => !met(password)).map(([rule]) => rule)

/** Why `password` cannot be the new master password, if it cannot. An empty
 *  one is the caller's to judge: a key file alone is allowed. */
export function masterPasswordProblem(password: string): string | null {
  const unmet = unmetRules(password)
  return password && unmet.length ? `The master password needs: ${unmet.map((r) => r.toLowerCase()).join('; ')}` : null
}

/** The rules as a tip next to the new password, shown before anything is
 *  typed; each one is ticked once the typed password meets it. */
export function masterRulesTip(password: HTMLInputElement): HTMLElement {
  const items = RULES.map(([rule]) => el('li', {}, rule))
  const refresh = () => RULES.forEach(([, met], i) => items[i].classList.toggle('met', met(password.value)))
  password.addEventListener('input', refresh)
  refresh()
  return el('div', { className: 'master-rules' }, el('span', {}, 'A new master password needs:'), el('ul', {}, ...items))
}
