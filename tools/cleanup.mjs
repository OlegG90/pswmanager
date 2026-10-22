// `npm run cleanup`: removes what the builds generate, so the checkout holds
// only the sources again. Dependencies (node_modules, the Python venv of
// sic2kdbx) and local data (databases, pswm.json, docs/security-audit) stay:
// they are not rebuilt by a build.
import { rmSync, existsSync } from 'node:fs'
import { join } from 'node:path'

const root = join(import.meta.dirname, '..')
const android = 'apps/android/gen/android'
const generated = [
  'target', // cargo: every Rust build, Windows and Android
  'src-tauri/target', // cargo before the workspace moved to the root
  'src-tauri/gen', // Tauri's schemas
  'dist', // vite: the Windows UI
  'dist-android', // vite: the Android UI
  'apps/android/gen/schemas',
  `${android}/.gradle`,
  `${android}/build`,
  `${android}/app/build`,
  `${android}/buildSrc/.gradle`,
  `${android}/buildSrc/build`,
  `${android}/app/src/main/java/io/github/olegg90/pswmanager/generated`,
  'tools/sic2kdbx/__pycache__',
  'tools/sic2kdbx/tests/__pycache__',
]

for (const path of generated) {
  const full = join(root, path)
  if (!existsSync(full)) continue
  rmSync(full, { recursive: true, force: true })
  console.log(`removed ${path}`)
}
