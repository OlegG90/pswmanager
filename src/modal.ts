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
      list.setAttribute('role', 'listbox')
      input.setAttribute('role', 'combobox')
      input.setAttribute('aria-autocomplete', 'list')
      input.setAttribute('aria-haspopup', 'listbox')
      input.setAttribute('aria-controls', list.id)
      input.setAttribute('aria-expanded', 'false')
      let activeIndex = -1
      const selectSuggestion = (suggestion: string) => {
        input.value = suggestion
        list.hidden = true
        activeIndex = -1
        for (const option of list.querySelectorAll('[role="option"]')) {
          option.setAttribute('aria-selected', 'false')
        }
        input.setAttribute('aria-expanded', 'false')
        input.removeAttribute('aria-activedescendant')
        input.focus()
        input.setSelectionRange(suggestion.length, suggestion.length)
      }
      const setActive = (index: number) => {
        const options = [...list.querySelectorAll<HTMLElement>('[role="option"]')]
        activeIndex = index
        for (let i = 0; i < options.length; i++) {
          options[i].setAttribute('aria-selected', String(i === index))
        }
        const active = options[index]
        if (active) input.setAttribute('aria-activedescendant', active.id)
        else input.removeAttribute('aria-activedescendant')
      }
      const showSuggestions = () => {
        const query = input.value.trim().toLocaleLowerCase()
        const matches = query ? suggestions.filter((s) => s.toLocaleLowerCase().includes(query)) : []
        list.replaceChildren(...matches.map((suggestion, i) => {
          const option = el('div', { id: `ask-suggestion-${i}`, className: 'suggestion', onclick: () => selectSuggestion(suggestion) }, suggestion)
          option.setAttribute('role', 'option')
          option.setAttribute('aria-selected', 'false')
          return option
        }))
        list.hidden = matches.length === 0
        input.setAttribute('aria-expanded', String(matches.length > 0))
        setActive(-1)
      }
      input.addEventListener('input', showSuggestions)
      input.addEventListener('keydown', (e) => {
        const optionCount = list.querySelectorAll('[role="option"]').length
        if (e.key === 'ArrowDown' && optionCount) {
          e.preventDefault()
          setActive(Math.min(activeIndex + 1, optionCount - 1))
        } else if (e.key === 'ArrowUp' && activeIndex >= 0) {
          e.preventDefault()
          setActive(activeIndex - 1)
        } else if (e.key === 'Enter' && activeIndex >= 0) {
          const option = list.querySelectorAll<HTMLElement>('[role="option"]')[activeIndex]
          if (!option) return
          e.preventDefault()
          e.stopImmediatePropagation()
          selectSuggestion(option.textContent ?? '')
        }
      })
      showSuggestions()
    }
    const confirm = button(confirmLabel, confirmLabel, () => answer(input.value), 'primary')
    enterPresses(confirm, input)
    requestAnimationFrame(() => input.setSelectionRange(0, selected))
    return { body: [input, ...(list ? [list] : [])], buttons: [confirm], focus: input }
  })
}
