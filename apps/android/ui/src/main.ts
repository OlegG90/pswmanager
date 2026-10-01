import { invoke } from '@tauri-apps/api/core'

interface About {
  version: string
  sample: string
}

const show = async () => {
  try {
    const about = await invoke<About>('about')
    document.querySelector('#version')!.textContent = `Version ${about.version}`
    document.querySelector('#sample')!.textContent = about.sample
  } catch (e) {
    document.querySelector('#sample')!.textContent = String(e)
  }
}

document.querySelector('#again')!.addEventListener('click', show)
void show()
