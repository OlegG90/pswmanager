import { button, el } from './dom'
import { parseTags } from './entry-text'

/**
 * Tags as chips, each with its remove button, and an input to add more:
 * Enter, a comma or leaving the input adds what was typed (commas and
 * semicolons separate several); Backspace in the empty input removes the last
 * chip. `known` are the database's tags, offered as the user types.
 */
export function tagInput(initial: string[], known: string[]): { element: HTMLElement; value: () => string[] } {
  let tags = [...initial]
  const listId = 'tag-list'
  const entry = el('input', { placeholder: 'Add a tag', spellcheck: false, ariaLabel: 'Add a tag' })
  entry.setAttribute('list', listId)
  const chips = el('span', { className: 'tag-chips' })
  const suggestions = el('datalist', { id: listId })
  const element = el('div', { className: 'tag-input', onclick: () => entry.focus() }, chips, entry, suggestions)

  const draw = () => {
    chips.replaceChildren(
      ...tags.map((tag) =>
        el('span', { className: 'tag' }, tag, button('×', `Remove the tag ${tag}`, () => {
          tags = tags.filter((t) => t !== tag)
          draw()
          entry.focus()
        }, 'ghost'))),
    )
    suggestions.replaceChildren(...known.filter((t) => !tags.includes(t)).map((t) => new Option(t)))
  }
  const add = () => {
    const added = parseTags(entry.value).filter((t) => !tags.includes(t))
    entry.value = ''
    if (added.length) {
      tags = [...tags, ...added]
      draw()
    }
  }

  entry.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ',' || e.key === ';') {
      // Enter would otherwise submit the form; with nothing typed it still does.
      if (e.key !== 'Enter' || entry.value.trim()) {
        e.preventDefault()
        add()
      }
    } else if (e.key === 'Backspace' && !entry.value && tags.length) {
      tags = tags.slice(0, -1)
      draw()
    }
  })
  // Picking a suggestion from the list puts it in the input: take it at once.
  entry.addEventListener('input', (e) => {
    if ((e as InputEvent).inputType === 'insertReplacementText' || !(e instanceof InputEvent)) add()
  })
  entry.addEventListener('blur', add)
  draw()
  // What was typed but not added yet counts too.
  return { element, value: () => [...tags, ...parseTags(entry.value).filter((t) => !tags.includes(t))] }
}
