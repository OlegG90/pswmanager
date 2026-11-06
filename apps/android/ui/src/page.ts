import { el, button } from '../../../../src/dom'
import type { Settings } from './api'
import { appState } from './app-state'
import { svgIcon, type IconName } from './icons'
import { screens as screensOf } from './screens'

// The page every screen draws on: the screen shown, the panels and overlays
// over it, the snackbar, and what the screens share (see app-state.ts).

export const screen = document.querySelector<HTMLElement>('#screen')!
const snackbar = document.querySelector<HTMLElement>('#snackbar')!

/** What the screens share. */
export const app = appState()

/** The screen shown, what it runs and what Back does there. */
export const screens = screensOf()

/** Called by Android's Back: true when the page did something with it. */
;(window as unknown as { pswmBack: () => boolean }).pswmBack = () => screens.back()

/** Shows a screen; `back` is where Back returns from it (none: Back leaves the app). */
export function show(children: Node[], back?: () => void, className = '') {
  screens.show(back)
  document.querySelectorAll('.shade, .sheet').forEach((n) => n.remove())
  screen.className = className
  screen.replaceChildren(...children)
  window.scrollTo(0, 0)
}

/** Puts an overlay (a panel, the drawer) on top: Back closes it. Returns its
 *  close; `closed` runs once it is closed, however. */
export function overlay(node: HTMLElement, closed: () => void = () => {}): () => void {
  const close = screens.overlay(() => node.remove(), closed)
  node.addEventListener('click', (e) => e.target === node && close())
  document.body.append(node)
  return close
}

let snackTimer = 0
export function snack(text: string) {
  snackbar.textContent = text
  snackbar.hidden = false
  clearTimeout(snackTimer)
  snackTimer = window.setTimeout(() => (snackbar.hidden = true), 3000)
}

/** A panel from the bottom; `fill` gets what closes it. */
export function sheet(fill: (close: () => void) => Node[], closed?: () => void) {
  const shade = el('div', { className: 'sheet' })
  const close = overlay(shade, closed)
  shade.append(el('div', { className: 'panel' }, ...fill(close)))
}

/** Asks before `action`, which runs on `yes`. */
export function confirmSheet(question: string, yes: string, action: () => void) {
  sheet((close) => [el('p', {}, question), button(yes, yes, () => (close(), action()), 'primary'), button('Cancel', 'Cancel', close, 'link')])
}

export async function copied(copy: Promise<number>) {
  try {
    snack(`Copied · clears in ${await copy} s`)
  } catch (e) {
    snack(String(e))
  }
}

/** Takes the settings: the theme now, the timings where they are used. */
export function applySettings(fresh: Settings) {
  app.settings = fresh
  if (fresh.theme === 'system') delete document.documentElement.dataset.theme
  else document.documentElement.dataset.theme = fresh.theme
}

/** A toolbar button showing an icon. */
export function iconButton(name: IconName, title: string, onClick: () => void, className = 'icon') {
  const b = button('', title, onClick, className)
  b.setAttribute('aria-label', title)
  b.append(svgIcon(name))
  return b
}
