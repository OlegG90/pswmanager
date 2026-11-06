import { listen } from '@tauri-apps/api/event'
import { api, type Synced } from './api'
import { AGAIN_AFTER } from './app-state'
import { chooseScreen } from './first-run'
import { listScreen } from './list'
import { app, applySettings, screens } from './page'
import { applySync } from './sync'
import { afterLock, lock, unlockedWith, unlockScreen } from './unlock'

// The phone's page: the first screen, and what runs beside every screen (the
// syncs and the locking on their own). The screens are in their own modules.

async function start() {
  applySettings(await api.settings())
  const status = await api.status()
  app.database = status.database
  app.unlocked = status.unlocked
  // In front now: a lock the credential provider set waits no longer (provider.rs).
  // Hidden meanwhile, the page has told the backend so already, and must not undo that.
  if (!document.hidden) void api.stayUnlocked()
  if (!app.database) chooseScreen()
  else if (status.unlocked) listScreen(await api.listing())
  else unlockScreen()
}

/** Shows the unlock screen after the database locked by itself (in the background). */
void listen('locked', () => {
  app.locked(false)
  unlockScreen()
})

// ------------------------------------------------------------ sync

void listen<Synced>('synced', (e) => applySync(e.payload))

/** Syncs on its own while unlocked: coming back to the app after a minute
 *  (AGAIN_AFTER), and as often as the setting says while it is in front;
 *  going away sends what is waiting. */
const syncIfOlder = (age: number) => {
  if (!document.hidden && app.syncDue(age)) void api.syncNow()
}
document.addEventListener('visibilitychange', () => {
  if (document.hidden) {
    app.wentAway()
    if (app.unlocked) void api.syncIfPending()
    // Locked too: the backend must know the app is no longer in front, or an unlock
    // by the credential provider meanwhile would never lock again (provider.rs).
    void api.lockLater(app.unlocked)
  } else {
    // The backend's timer may not have run on time (Android asleep): the clock decides.
    const lockNow = app.cameBack()
    // Unlocked meanwhile for a passkey (the credential provider shares the session).
    if (!app.unlocked && app.database) void api.status().then(async (s) => (s.unlocked ? unlockedWith(await api.listing()) : screens.runReturn()), () => screens.runReturn())
    else screens.runReturn()
    void api.stayUnlocked()
    if (lockNow) void lock()
    else syncIfOlder(AGAIN_AFTER)
  }
})
setInterval(() => !document.hidden && app.everySyncDue() && void api.syncNow(), 30 * 1000)

// ------------------------------------------------------------ locking

for (const type of ['pointerdown', 'keydown', 'scroll']) document.addEventListener(type, () => app.touched(), { passive: true, capture: true })
setInterval(() => !document.hidden && app.idle() && void lock(), 10 * 1000)
/** The screen turned off (SystemPlugin.kt): the backend locks when the setting says so. */
;(window as unknown as { pswmScreenOff: () => void }).pswmScreenOff = () => {
  if (app.unlocked) void api.screenOff().then((locked) => locked && afterLock())
}

void start()
