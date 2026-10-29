import { describe, expect, it, vi } from 'vitest'
import type { Synced } from './api'
import { Screens } from './screens'

const synced = { text: 'Synced', changed: true } as Synced

describe('Screens', () => {
  it('goes back to where a screen came from, then leaves the app', () => {
    const screens = new Screens()
    const toList = vi.fn()
    screens.show(toList)
    expect(screens.back()).toBe(true)
    expect(toList).toHaveBeenCalledOnce()
    expect(screens.back()).toBe(false)
    screens.show()
    expect(screens.back()).toBe(false)
  })

  it('undoes the innermost step first', () => {
    const screens = new Screens()
    const order: string[] = []
    screens.show(() => order.push('screen'))
    const closeSearch = () => order.push('search')
    screens.push(closeSearch)
    screens.push(closeSearch)
    screens.overlay(() => order.push('sheet'))
    while (screens.back());
    expect(order).toEqual(['sheet', 'search', 'screen'])
  })

  it('forgets a step dropped', () => {
    const screens = new Screens()
    const clearSelection = vi.fn()
    screens.show()
    screens.push(clearSelection)
    screens.drop(clearSelection)
    expect(screens.back()).toBe(false)
    expect(clearSelection).not.toHaveBeenCalled()
  })

  it('closes an overlay once, from Back or its own close', () => {
    const screens = new Screens()
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
    const screens = new Screens()
    const [stop, onSynced, onReturn, overlayBack] = [vi.fn(), vi.fn(), vi.fn(), vi.fn()]
    screens.show()
    const first = screens.current
    screens.onLeave(stop)
    screens.onSynced(onSynced)
    screens.onReturn(onReturn)
    screens.synced(synced)
    screens.returned()
    expect(onSynced).toHaveBeenCalledWith(synced)
    expect(onReturn).toHaveBeenCalledOnce()
    screens.overlay(overlayBack)
    screens.show()
    expect(screens.current).not.toBe(first)
    expect(stop).toHaveBeenCalledOnce()
    screens.synced(synced)
    screens.returned()
    expect(onSynced).toHaveBeenCalledOnce()
    expect(onReturn).toHaveBeenCalledOnce()
    expect(screens.back()).toBe(false)
    expect(overlayBack).not.toHaveBeenCalled()
    screens.show()
    expect(stop).toHaveBeenCalledOnce()
  })
})
