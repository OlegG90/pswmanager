import { api, type IconChoice } from './api'
import { button, el } from './dom'
import { GLYPHS, glyphIcon } from './glyphs'

/** An image kept in the database, as the browser can show it. */
export const imageUrl = (data: string) => `data:image/png;base64,${data}`

/**
 * The editor's icon choice: Auto (the site's icon, else the key), one of the
 * app's drawings, or an image file kept in the database. `autoPreview` is
 * what Auto shows for this entry now.
 */
export function iconPicker(
  initial: IconChoice,
  autoPreview: string,
  onError: (message: string) => void,
): { element: HTMLElement; value: () => IconChoice } {
  let choice = initial
  const preview = el('img', { className: 'icon', alt: '' })
  const panel = el('div', { className: 'icon-choices', hidden: true })
  const tile = (src: string, title: string, chosen: boolean, pick: () => void) => {
    const b = button('', title, pick, 'icon-tile')
    b.setAttribute('aria-pressed', String(chosen))
    b.append(el('img', { src, alt: '' }))
    return b
  }

  const draw = () => {
    preview.src = choice.kind === 'builtin' ? glyphIcon(choice.id) : choice.kind === 'custom' ? imageUrl(choice.data) : autoPreview
    panel.replaceChildren(
      tile(autoPreview, 'Auto: the site\'s icon, or the key', choice.kind === 'auto', () => set({ kind: 'auto' })),
      ...GLYPHS.filter((g) => g.id !== 0).map((g) =>
        tile(glyphIcon(g.id), g.name, choice.kind === 'builtin' && choice.id === g.id, () => set({ kind: 'builtin', id: g.id }))),
      ...(choice.kind === 'custom' ? [tile(imageUrl(choice.data), 'This entry\'s image', true, () => {})] : []),
      button('Image…', 'An image file (PNG, JPEG, GIF or WebP, up to 256 KB), kept in the database', async () => {
        try {
          const data = await api.pickIconImage()
          if (data) set({ kind: 'custom', data })
        } catch (e) {
          onError(String(e))
        }
      }, 'ghost'),
    )
  }
  const set = (next: IconChoice) => {
    choice = next
    draw()
  }
  const toggle = button('Change…', 'Choose the icon', () => {
    panel.hidden = !panel.hidden
  })
  draw()
  return { element: el('div', { className: 'icon-picker' }, el('div', { className: 'controls' }, preview, toggle), panel), value: () => choice }
}
