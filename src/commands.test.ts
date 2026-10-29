/// <reference types="node" />
// A command is declared three times: a `#[tauri::command]` function, an entry
// in `generate_handler!` and an `invoke('name')` in the UI. Nothing in the
// build ties the last two together, so a renamed or forgotten command would
// only fail when a screen calls it (#317). These tests read the sources and
// hold the handler list and the commands the UI calls to the same set.
import { readFileSync, readdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const root = resolve(import.meta.dirname, '..')
const read = (file: string) => readFileSync(join(root, file), 'utf8')

/** The commands in the `generate_handler!` list of `file`, without their module paths. */
function handlers(file: string): string[] {
  // Without comments and attributes: an entry's `#[cfg(…)]` would end the list at its own bracket.
  const text = read(file)
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/\/\/.*$/gm, '')
    .replace(/#\[[^\]]*\]/g, '')
  const open = 'generate_handler!['
  const start = text.indexOf(open)
  if (start < 0) throw new Error(`no generate_handler! in ${file}`)
  const names = text
    .slice(start + open.length, text.indexOf(']', start))
    .split(',')
    .map(path => path.trim().split('::').pop()!)
    .filter(name => name !== '')
  expect(names.length, `the handler list in ${file}`).toBeGreaterThan(0)
  return names
}

/** The commands the TypeScript in `files` invokes. Every call names its command literally. */
function invoked(files: string[]): string[] {
  const names = new Set<string>()
  for (const file of files) {
    const text = read(file)
    const calls = text.match(/\binvoke\s*[<(]/g)?.length ?? 0
    const named = [...text.matchAll(/\binvoke\s*(?:<[^(]*>)?\(\s*'([a-z_]+)'/g)].map(m => m[1])
    expect(named.length, `every invoke in ${file} names its command`).toBe(calls)
    named.forEach(name => names.add(name))
  }
  return [...names]
}

/** What differs between the commands `ui` invokes and the `handled` ones, by name: nothing when they agree. */
function drift(ui: string[], handled: string[]) {
  return {
    unhandled: ui.filter(name => !handled.includes(name)),
    neverInvoked: handled.filter(name => !ui.includes(name)),
  }
}
const NO_DRIFT = { unhandled: [], neverInvoked: [] }

const SHARED_API = 'src/api.ts'

/**
 * The relative imports and re-exports of `file`, as paths from the root, and
 * whether each brings the value `api` (by name, renamed, or in a namespace).
 * The clause before `from` holds no quote, so one match never runs into the
 * statement before it.
 */
function imports(file: string): { path: string; api: boolean }[] {
  return [...read(file).matchAll(/^(?:import|export)\s+(type\s+)?([^'"]*?)\s+from\s+'([^']+)'/gm)]
    .filter(m => m[3].startsWith('.') && !m[3].endsWith('.css'))
    .map(m => ({
      path: join(dirname(file), `${m[3]}.ts`).replaceAll('\\', '/'),
      api: !m[1] && (m[2].includes('*') || /(^|[{,])\s*api\b/.test(m[2])),
    }))
}

/** The TypeScript files the app starting at `entry` bundles. */
function reachable(entry: string): string[] {
  const seen = new Set([entry])
  for (const file of seen) imports(file).forEach(({ path }) => seen.add(path))
  return [...seen]
}

describe('Windows commands', () => {
  it('are the commands the UI invokes', () => {
    const files = readdirSync(join(root, 'src'), { recursive: true, encoding: 'utf8' })
      .filter(name => name.endsWith('.ts') && !name.endsWith('.test.ts'))
      .map(name => `src/${name.replaceAll('\\', '/')}`)
    expect(drift(invoked(files), handlers('src-tauri/src/lib.rs'))).toEqual(NO_DRIFT)
  })
})

describe('Android commands', () => {
  const files = reachable('apps/android/ui/src/main.ts')

  it('are the commands the UI invokes', () => {
    // The shared `src/api.ts` is bundled for its types and field names only:
    // its `api` wraps the Windows commands.
    expect(drift(invoked(files.filter(file => file !== SHARED_API)), handlers('apps/android/src/app.rs'))).toEqual(NO_DRIFT)
  })

  it('leave the Windows wrappers alone', () => {
    const windowsApiUsers = files.filter(file => imports(file).some(({ path, api }) => path === SHARED_API && api))
    expect(windowsApiUsers).toEqual([])
  })
})
