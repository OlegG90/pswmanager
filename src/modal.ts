import { button, el, enterPresses } from './dom'

/** True while a question is on screen: the page's own keys wait. */
export const isAsking = () => document.querySelector('dialog[open]') !== null

/**
 * Shows `message` in a modal dialog with what `build` adds, and resolves to
 * what the user picked. The Cancel button gives `cancelled`, has the focus and is what
 * Esc gives, so a key pressed by habit never loses anything. `className`
 * styles the box beside `modal`.
 * (`window.confirm` cannot be used: in this webview it answers "yes" unasked.)
 */
export function dialog<T>(
  message: string,
  cancelled: T,
  build: (answer: (value: T) => void) => { body?: Node[]; buttons?: Node[]; focus?: HTMLElement },
  cancelLabel = 'Cancel',
  className = '',
): Promise<T> {
  return new Promise((resolve) => {
    const box = el('dialog', { className: `modal ${className}`.trim() })
    const answer = (value: T) => {
      box.close()
      box.remove()
      resolve(value)
    }
    const cancel = button(cancelLabel, `${cancelLabel} (Esc)`, () => answer(cancelled))
    const { body = [], buttons = [], focus = cancel } = build(answer)
    box.append(el('p', {}, message), ...body, el('div', { className: 'buttons' }, ...buttons, cancel))
    // Esc closes a modal dialog by itself; make that the safe answer.
    box.addEventListener('cancel', (e) => {
      e.preventDefault()
      answer(cancelled)
    })
    document.body.append(box)
    box.showModal()
    focus.focus()
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

/** How much of a file name to select for renaming: the name without its
 *  extension, as Explorer does. */
export function beforeExtension(name: string): number {
  const dot = name.lastIndexOf('.')
  return dot > 0 ? dot : name.length
}

/** Asks for a line of text, starting from `value` with `selected` of it
 *  selected, offering `suggestions` as it is typed; resolves to the text, or
 *  `null` for Cancel. Enter confirms. */
export function askText(
  message: string,
  value: string,
  confirmLabel: string,
  selected = value.length,
  suggestions: string[] = [],
): Promise<string | null> {
  return dialog<string | null>(message, null, (answer) => {
    const input = el('input', { value, spellcheck: false, className: 'text' })
    const list = el('datalist', { id: 'ask-suggestions' }, ...suggestions.map((s) => new Option(s)))
    input.setAttribute('list', list.id)
    const confirm = button(confirmLabel, confirmLabel, () => answer(input.value), 'primary')
    enterPresses(confirm, input)
    requestAnimationFrame(() => input.setSelectionRange(0, selected))
    return { body: [input, list], buttons: [confirm], focus: input }
  })
}
