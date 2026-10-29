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
export class Screens {
  /** What Back undoes, innermost last. */
  private steps: (() => void)[] = []
  private shown = 0
  private stop = () => {}
  private answerSynced = (_synced: Synced) => {}
  private answerReturn = () => {}

  /** A new screen: `back` is where Back returns from it (none: Back leaves the app). */
  show(back?: () => void) {
    this.shown++
    this.stop()
    this.stop = () => {}
    this.answerSynced = () => {}
    this.answerReturn = () => {}
    this.steps = back ? [back] : []
  }

  /** Which screen is shown: another number later means another screen came meanwhile. */
  get current(): number {
    return this.shown
  }

  /** Back undoes `step` first, until it is dropped or undone. */
  push(step: () => void) {
    if (!this.steps.includes(step)) this.steps.push(step)
  }

  drop(step: () => void) {
    this.steps = this.steps.filter((s) => s !== step)
  }

  /** Android's Back: true when the page did something with it. */
  back(): boolean {
    const step = this.steps.pop()
    step?.()
    return !!step
  }

  /**
   * An overlay on top of the screen (a panel, the drawer), which Back closes
   * first. Returns its close, which `remove`s it from the page; `closed` runs
   * once it is closed, however. A new screen takes overlays away without it.
   */
  overlay(remove: () => void, closed: () => void = () => {}): () => void {
    const close = () => {
      remove()
      this.drop(close)
      closed()
    }
    this.push(close)
    return close
  }

  /** Stops what the screen shown runs on a timer (the TOTP countdown) once another comes. */
  onLeave(stop: () => void) {
    this.stop = stop
  }

  /** The screen shown's answer to a sync (the list's status line). */
  onSynced(answer: (synced: Synced) => void) {
    this.answerSynced = answer
  }

  synced(synced: Synced) {
    this.answerSynced(synced)
  }

  /** The screen shown's answer to coming back from another app (the unlock screen's prompt). */
  onReturn(answer: () => void) {
    this.answerReturn = answer
  }

  returned() {
    this.answerReturn()
  }
}
