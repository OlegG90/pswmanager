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
