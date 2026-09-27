import { button, el } from './dom'

/** True while a question is on screen: the page's own keys wait. */
export const isAsking = () => document.querySelector('dialog[open]') !== null

/**
 * Shows `message` in a modal dialog with what `build` adds, and resolves to
 * what the user picked. The Cancel button gives `cancelled`, has the focus and is what
 * Esc gives, so a key pressed by habit never loses anything.
 * (`window.confirm` cannot be used: in this webview it answers "yes" unasked.)
 */
function dialog<T>(
  message: string,
  cancelled: T,
  build: (answer: (value: T) => void) => { body?: Node[]; buttons?: Node[] },
  cancelLabel = 'Cancel',
): Promise<T> {
  return new Promise((resolve) => {
    const box = el('dialog', { className: 'modal' })
    const answer = (value: T) => {
      box.close()
      box.remove()
      resolve(value)
    }
    const cancel = button(cancelLabel, `${cancelLabel} (Esc)`, () => answer(cancelled))
    const { body = [], buttons = [] } = build(answer)
    box.append(el('p', {}, message), ...body, el('div', { className: 'buttons' }, ...buttons, cancel))
    // Esc closes a modal dialog by itself; make that the safe answer.
    box.addEventListener('cancel', (e) => {
      e.preventDefault()
      answer(cancelled)
    })
    document.body.append(box)
    box.showModal()
    cancel.focus()
  })
}

/** Asks a yes / no question; the confirming button is marked as dangerous. */
export function ask(message: string, confirmLabel: string, cancelLabel = 'Cancel'): Promise<boolean> {
  return dialog(message, false, (answer) => ({ buttons: [button(confirmLabel, confirmLabel, () => answer(true), 'danger')] }), cancelLabel)
}

/** Asks to pick one of `choices`; resolves to its index, or `null` for Cancel. */
export function choose(message: string, choices: string[]): Promise<number | null> {
  return dialog<number | null>(message, null, (answer) => ({
    body: [el('div', { className: 'choices' }, ...choices.map((label, i) => button(label, label, () => answer(i))))],
  }))
}
