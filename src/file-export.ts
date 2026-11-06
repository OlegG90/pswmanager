import { api, type ExportFormat } from './api'
import { busyButton, button, choiceList, el, enterPresses } from './dom'
import { EXPORT_FORMATS, EXPORT_INTRO, EXPORT_WARNING, exportedReport } from './file-export-parts'

/**
 * Tools › Export to a file (#240), beside the sidebar: the format, the user
 * checked again (Windows Hello, or the master password typed here), the file
 * named in the save dialog, then what was written and what was left out.
 */
export function renderExport(container: HTMLElement, helloReady: boolean, fail: (message: string) => void) {
  // The page's own element: once the page is left (Esc, another choice in the
  // sidebar), a check or a save dialog still open must not go on to write.
  const page = el('div')
  container.replaceChildren(page)
  const left = () => !page.isConnected
  // A step in a card (`tool`), or the formats' list (`choose`).
  const showIn = (className: string, ...body: Node[]) => {
    page.replaceChildren(el('header', {}, el('h1', {}, 'Export to a file')), el('section', { className }, ...body))
    page.querySelector<HTMLElement>('section input, section button')?.focus()
  }
  const show = (...body: Node[]) => showIn('tool', ...body)
  const actions = (...buttons: HTMLElement[]) => el('div', { className: 'tool-actions' }, ...buttons)

  // The formats as a list, each its button with what it carries (#353).
  const start = (): void =>
    showIn('choose',
      el('p', {}, EXPORT_INTRO),
      choiceList(EXPORT_FORMATS.map(([format, name, what]) => [busyButton(name, `Export as ${name}`, () => chosen(format), fail), what])),
      el('p', { className: 'muted' }, EXPORT_WARNING))

  // Windows Hello first; when it cannot, the master password.
  const chosen = async (format: ExportFormat): Promise<void> => {
    let confirmed = false
    if (helloReady) {
      try {
        await api.confirmExport(null)
        confirmed = true
      } catch (e) {
        if (String(e) === 'cancelled') return
      }
    }
    if (left()) return
    if (confirmed) await write(format)
    else askPassword(format, '')
  }

  const askPassword = (format: ExportFormat, wrong: string): void => {
    const input = el('input', { type: 'password', className: 'text', ariaLabel: 'Master password' })
    const ok = busyButton('Export', 'Export', async () => {
      try {
        await api.confirmExport(input.value)
      } catch (e) {
        if (!left()) askPassword(format, String(e))
        return
      }
      if (!left()) await write(format)
    }, fail, 'primary')
    enterPresses(ok, input)
    show(el('p', {}, 'Enter the master password to export:'), input,
      ...(wrong ? [el('p', { className: 'error' }, wrong)] : []),
      actions(ok, button('Back', 'Choose another format', start)))
  }

  const write = async (format: ExportFormat): Promise<void> => {
    const exported = await api.exportFile(format)
    if (left()) return
    if (!exported) return start()
    show(...exportedReport(exported), actions(button('Export again', 'Choose a format again', start)))
  }

  start()
}
