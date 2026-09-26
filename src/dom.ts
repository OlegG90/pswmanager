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
