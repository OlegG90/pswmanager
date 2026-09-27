import { button, el } from './dom'

/** True while a question is on screen: the page's own keys wait. */
export const isAsking = () => document.querySelector('dialog[open]') !== null

/**
 * Asks in a modal dialog. The safe answer has the focus and is what Esc
 * gives, so a key pressed by habit never loses anything.
 * (`window.confirm` cannot be used: in this webview it answers "yes" unasked.)
 */
export function ask(message: string, confirmLabel: string, cancelLabel = 'Cancel'): Promise<boolean> {
  return new Promise((resolve) => {
    const dialog = el('dialog', { className: 'modal' })
    const answer = (yes: boolean) => {
      dialog.close()
      dialog.remove()
      resolve(yes)
    }
    const cancel = button(cancelLabel, `${cancelLabel} (Esc)`, () => answer(false))
    dialog.append(
      el('p', {}, message),
      el('div', { className: 'buttons' }, button(confirmLabel, confirmLabel, () => answer(true), 'danger'), cancel),
    )
    // Esc closes a modal dialog by itself; make that the safe answer.
    dialog.addEventListener('cancel', (e) => {
      e.preventDefault()
      answer(false)
    })
    document.body.append(dialog)
    dialog.showModal()
    cancel.focus()
  })
}

/**
 * Asks to pick one of `choices` in a modal dialog; resolves to its index, or
 * `null` for Cancel (which has the focus, and is what Esc gives).
 */
export function choose(message: string, choices: string[]): Promise<number | null> {
  return new Promise((resolve) => {
    const dialog = el('dialog', { className: 'modal' })
    const answer = (index: number | null) => {
      dialog.close()
      dialog.remove()
      resolve(index)
    }
    const cancel = button('Cancel', 'Cancel (Esc)', () => answer(null))
    dialog.append(
      el('p', {}, message),
      el('div', { className: 'choices' }, ...choices.map((label, i) => button(label, label, () => answer(i)))),
      el('div', { className: 'buttons' }, cancel),
    )
    dialog.addEventListener('cancel', (e) => {
      e.preventDefault()
      answer(null)
    })
    document.body.append(dialog)
    dialog.showModal()
    cancel.focus()
  })
}
