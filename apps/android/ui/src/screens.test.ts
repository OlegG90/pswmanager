import { describe, expect, it, vi } from 'vitest'
import type { Synced } from './api'
import { screens as screensOf } from './screens'

const synced = { text: 'Synced', changed: true } as Synced

describe('Screens', () => {
  it('goes back to where a screen came from, then leaves the app', () => {
    const screens = screensOf()
    const toList = vi.fn()
    screens.show(toList)
    expect(screens.back()).toBe(true)
    expect(toList).toHaveBeenCalledOnce()
    expect(screens.back()).toBe(false)
    screens.show()
    expect(screens.back()).toBe(false)
  })

  it('undoes the innermost step first', () => {
    const screens = screensOf()
    const order: string[] = []
    screens.show(() => order.push('screen'))
    const closeSearch = () => order.push('search')
    // Pushed twice, undone once: a step is on the stack at most once.
    screens.push(closeSearch)
    screens.push(closeSearch)
    screens.overlay(() => order.push('sheet'))
    while (screens.back());
    expect(order).toEqual(['sheet', 'search', 'screen'])
  })

  it('keeps a step that puts itself back when undone', () => {
    // The editor's Back asks before leaving and stays when the changes are kept.
    const screens = screensOf()
    const leaveEditor = vi.fn()
    screens.show(leaveEditor)
    const ask = vi.fn()
    const onBack = () => {
      screens.push(onBack)
      ask()
    }
    screens.push(onBack)
    screens.back()
    screens.back()
    expect(ask).toHaveBeenCalledTimes(2)
    expect(leaveEditor).not.toHaveBeenCalled()
  })

  it('forgets a step dropped', () => {
    const screens = screensOf()
    const clearSelection = vi.fn()
    screens.show()
    screens.push(clearSelection)
    screens.drop(clearSelection)
    expect(screens.back()).toBe(false)
    expect(clearSelection).not.toHaveBeenCalled()
  })

  it('closes an overlay once, from Back or its own close', () => {
    const screens = screensOf()
    const back = vi.fn()
    screens.show(back)
    const [remove, closed] = [vi.fn(), vi.fn()]
    const close = screens.overlay(remove, closed)
    close()
    expect(remove).toHaveBeenCalledOnce()
    expect(closed).toHaveBeenCalledOnce()
    screens.back()
    expect(back).toHaveBeenCalledOnce()
    expect(remove).toHaveBeenCalledOnce()
  })

  it('stops what a screen ran and drops its answers when another comes', () => {
    const screens = screensOf()
    const [stop, onSynced, onReturn, overlayBack] = [vi.fn(), vi.fn(), vi.fn(), vi.fn()]
    screens.show()
    const first = screens.current
    screens.onLeave(stop)
    screens.onSynced(onSynced)
    screens.onReturn(onReturn)
    screens.runSynced(synced)
    screens.runReturn()
    expect(onSynced).toHaveBeenCalledWith(synced)
    expect(onReturn).toHaveBeenCalledOnce()
    screens.overlay(overlayBack)
    screens.show()
    expect(screens.current).not.toBe(first)
    expect(stop).toHaveBeenCalledOnce()
    screens.runSynced(synced)
    screens.runReturn()
    expect(onSynced).toHaveBeenCalledOnce()
    expect(onReturn).toHaveBeenCalledOnce()
    expect(screens.back()).toBe(false)
    expect(overlayBack).not.toHaveBeenCalled()
    screens.show()
    expect(stop).toHaveBeenCalledOnce()
  })
})
