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

export { beforeExtension } from './entry-text'

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
    const input = el('input', { value, spellcheck: false, className: `text${suggestions.length ? ' with-suggestions' : ''}` })
    const list = suggestions.length ? el('div', { id: 'ask-suggestions', className: 'suggestions', hidden: true }) : null
    if (list) {
      input.setAttribute('aria-autocomplete', 'list')
      input.setAttribute('aria-controls', list.id)
      input.setAttribute('aria-expanded', 'false')
      const showSuggestions = () => {
        const query = input.value.trim().toLocaleLowerCase()
        const matches = query ? suggestions.filter((s) => s.toLocaleLowerCase().includes(query)) : []
        list.replaceChildren(
          ...matches.map((suggestion) =>
            button(suggestion, `Use ${suggestion}`, () => {
              input.value = suggestion
              list.hidden = true
              input.setAttribute('aria-expanded', 'false')
              input.focus()
              input.setSelectionRange(suggestion.length, suggestion.length)
            }, 'suggestion')),
        )
        list.hidden = matches.length === 0
        input.setAttribute('aria-expanded', String(matches.length > 0))
      }
      input.addEventListener('input', showSuggestions)
      input.addEventListener('keydown', (e) => {
        if (e.key !== 'ArrowDown' || list.hidden) return
        e.preventDefault()
        list.querySelector<HTMLButtonElement>('button')?.focus()
      })
      list.addEventListener('keydown', (e) => {
        if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
        const items = [...list.querySelectorAll<HTMLButtonElement>('button')]
        const next = items.indexOf(document.activeElement as HTMLButtonElement) + (e.key === 'ArrowDown' ? 1 : -1)
        e.preventDefault()
        if (next < 0) input.focus()
        else items[next]?.focus()
      })
      showSuggestions()
    }
    const confirm = button(confirmLabel, confirmLabel, () => answer(input.value), 'primary')
    enterPresses(confirm, input)
    requestAnimationFrame(() => input.setSelectionRange(0, selected))
    return { body: [input, ...(list ? [list] : [])], buttons: [confirm], focus: input }
  })
}
