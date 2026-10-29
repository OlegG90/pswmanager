import type { Synced } from './api'

/**
 * The phone shows one screen at a time; this is what it runs beside its page
 * and what Android's Back does there (#321). A new screen stops what the one
 * before ran on a timer and drops its answers to a sync and to coming back
 * from another app. Back undoes the innermost step first (an overlay, an open
 * search, a selection), then returns to where the screen came from; with
 * nothing left it leaves the app (see SystemPlugin.kt). The page itself is
 * main.ts's: nothing here touches the DOM.
 */
export function screens() {
  /** What Back undoes, innermost last. */
  let steps: (() => void)[] = []
  let shown = 0
  let stop = () => {}
  let answerSynced = (_synced: Synced) => {}
  let answerReturn = () => {}

  const push = (step: () => void) => {
    if (!steps.includes(step)) steps.push(step)
  }
  const drop = (step: () => void) => {
    steps = steps.filter((s) => s !== step)
  }

  return {
    /** A new screen: `back` is where Back returns from it (none: Back leaves the app). */
    show(back?: () => void) {
      shown++
      stop()
      stop = () => {}
      answerSynced = () => {}
      answerReturn = () => {}
      steps = back ? [back] : []
    },

    /** Which screen is shown: another number later means another screen came meanwhile. */
    get current(): number {
      return shown
    },

    /** Back undoes `step` first, until it is dropped or undone. */
    push,
    drop,

    /** Android's Back: true when the page did something with it. */
    back(): boolean {
      const step = steps.pop()
      step?.()
      return !!step
    },

    /**
     * An overlay on top of the screen (a panel, the drawer), which Back closes
     * first. Returns its close, which `remove`s it from the page; `closed` runs
     * once it is closed, however. A new screen takes overlays away without it.
     */
    overlay(remove: () => void, closed: () => void = () => {}): () => void {
      const close = () => {
        remove()
        drop(close)
        closed()
      }
      push(close)
      return close
    },

    /** Stops what the screen shown runs on a timer (the TOTP countdown) once another comes. */
    onLeave(stopping: () => void) {
      stop = stopping
    },

    /** The screen shown's answer to a sync (the list's status line). */
    onSynced(answer: (synced: Synced) => void) {
      answerSynced = answer
    },
    runSynced(synced: Synced) {
      answerSynced(synced)
    },

    /** The screen shown's answer to coming back from another app (the unlock screen's prompt). */
    onReturn(answer: () => void) {
      answerReturn = answer
    },
    runReturn() {
      answerReturn()
    },
  }
}
