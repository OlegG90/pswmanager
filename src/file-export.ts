import { api, type ExportFormat } from './api'
import { button, el, enterPresses } from './dom'
import { EXPORT_FORMATS, exportedReport, fileExportAbout } from './file-export-parts'
import { dialog } from './modal'

/**
 * Tools › Export to a file… (#240): the format, the user checked again
 * (Windows Hello, or the master password), the file named in the save dialog,
 * then what was written and what was left out.
 */
export async function exportToFile(helloReady: boolean) {
  const format = await dialog<ExportFormat | null>('Export to a file', null, (answer) => ({
    body: fileExportAbout(),
    buttons: EXPORT_FORMATS.map(([format, name]) => button(name, `Export as ${name}`, () => answer(format), 'primary')),
  }))
  if (!format) return
  if (!(await confirm(helloReady))) return
  const exported = await api.exportFile(format)
  if (!exported) return
  await dialog('Exported', null, () => ({ body: exportedReport(exported) }), 'OK')
}

/** Windows Hello, or the master password: true once the backend allows the export. */
async function confirm(helloReady: boolean): Promise<boolean> {
  if (helloReady) {
    try {
      await api.confirmExport(null)
      return true
    } catch (e) {
      if (String(e) === 'cancelled') return false
      // Windows Hello could not: the master password instead.
    }
  }
  for (let wrong = ''; ;) {
    const password = await dialog<string | null>('Enter the master password to export', null, (answer) => {
      const input = el('input', { type: 'password', className: 'text', ariaLabel: 'Master password' })
      const ok = button('Export', 'Export', () => answer(input.value), 'primary')
      enterPresses(ok, input)
      return { body: [input, ...(wrong ? [el('p', { className: 'error' }, wrong)] : [])], buttons: [ok], focus: input }
    })
    if (password === null) return false
    try {
      await api.confirmExport(password)
      return true
    } catch (e) {
      wrong = String(e)
    }
  }
}
