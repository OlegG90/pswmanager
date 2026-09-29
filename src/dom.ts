/** Creates an element with the given properties and children. */
export function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: object = {}, ...children: (Node | string)[]) {
  const node: HTMLElementTagNameMap[K] = Object.assign(document.createElement(tag), props)
  node.append(...children)
  return node
}

/** A plain button (never a form's submit button). */
export function button(label: string, title: string, onClick: () => void, className = ''): HTMLButtonElement {
  return el('button', { type: 'button', title, className, onclick: onClick }, label)
}

/** The line under a form that says what went wrong, read out when it shows. */
export function errorLine() {
  const line = el('p', { className: 'error', role: 'alert', hidden: true })
  const hide = () => {
    line.hidden = true
  }
  return {
    line,
    show: (message: string) => {
      line.textContent = message
      line.hidden = false
    },
    hide,
    /** Hidden again as soon as `input` is typed in. */
    hideOnInput: (input: HTMLElement) => input.addEventListener('input', hide),
  }
}

/**
 * A button disabled while its `action` runs, so a second click cannot start
 * it again; what the action throws goes to `fail`.
 */
export function busyButton(label: string, title: string, action: () => Promise<unknown>, fail: (message: string) => void, className = '') {
  const pressed = button(label, title, async () => {
    pressed.disabled = true
    try {
      await action()
    } catch (e) {
      fail(String(e))
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
