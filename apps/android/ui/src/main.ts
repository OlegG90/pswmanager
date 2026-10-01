import { invoke } from '@tauri-apps/api/core'

interface About {
  version: string
  sample: string
}

const version = document.querySelector<HTMLElement>('#version')!
const sample = document.querySelector<HTMLElement>('#sample')!

const showAbout = async () => {
  try {
    const about = await invoke<About>('about')
    version.textContent = `Version ${about.version}`
    sample.textContent = about.sample
  } catch (e) {
    sample.textContent = String(e)
  }
}

document.querySelector('#again')!.addEventListener('click', showAbout)
void showAbout()

interface Picked {
  uri: string
  name: string
}

const picked = document.querySelector<HTMLElement>('#picked')!
const check = document.querySelector<HTMLButtonElement>('#check')!
const report = document.querySelector<HTMLElement>('#report')!
let folder: Picked | null = null

const pick = async (command: string, isFolder: boolean) => {
  try {
    const chosen = await invoke<Picked | null>(command)
    if (!chosen) return
    picked.textContent = `${isFolder ? 'Folder' : 'File'}: ${chosen.name}`
    folder = isFolder ? chosen : null
    check.hidden = !isFolder
  } catch (e) {
    picked.textContent = String(e)
  }
}

const showReport = (lines: string[]) => {
  report.replaceChildren(...lines.map((line) => Object.assign(document.createElement('li'), { textContent: line })))
}

document.querySelector('#folder')!.addEventListener('click', () => pick('pick_folder', true))
document.querySelector('#file')!.addEventListener('click', () => pick('pick_file', false))
check.addEventListener('click', async () => {
  if (!folder) return
  try {
    showReport(await invoke<string[]>('check_folder', { folder: folder.uri }))
  } catch (e) {
    showReport([String(e)])
  }
})
