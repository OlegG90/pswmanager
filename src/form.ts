import { button, el } from './dom'

/** The line under a form that says what went wrong, read out when it shows. */
export type ErrorLine = ReturnType<typeof errorLine>

export function errorLine() {
  const line = el('p', { className: 'error', role: 'alert', hidden: true })
  const hide = () => (line.hidden = true)
  return {
    line,
    show: (message: string) => {
      line.textContent = message
      line.hidden = false
    },
    hide,
    /** Hidden again as soon as any of `inputs` is typed in. */
    hideOnInput: (...inputs: HTMLElement[]) => {
      for (const input of inputs) input.addEventListener('input', hide)
    },
  }
}

/**
 * A button disabled while its `action` runs, so a second click cannot start
 * it again; what the action throws is shown in `error`.
 */
export function busyButton(label: string, title: string, action: () => Promise<unknown>, error: ErrorLine, className = '') {
  const pressed = button(label, title, async () => {
    pressed.disabled = true
    try {
      await action()
    } catch (e) {
      error.show(String(e))
    } finally {
      pressed.disabled = false
    }
  }, className)
  return pressed
}

/** Enter in any of `inputs` presses `target`, the form's main button. */
export function enterPresses(target: HTMLButtonElement, ...inputs: HTMLElement[]) {
  for (const input of inputs) {
    input.addEventListener('keydown', (e) => {
      if (e.key !== 'Enter') return
      e.preventDefault()
      target.click()
    })
  }
}
