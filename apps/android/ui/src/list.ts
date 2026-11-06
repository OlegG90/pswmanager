import { listen } from '@tauri-apps/api/event'
import { button, el, enterPresses } from '../../../../src/dom'
import { input } from '../../../../src/editor-parts'
import { titleOf } from '../../../../src/entry-text'
import { DEFAULT_ICON, glyphIcon } from '../../../../src/glyphs'
import { GROUP_ICONS } from '../../../../src/icons'
import { FAVORITE, type Filter, GROUPS, sameFilter, search, tagCounts, TEMPLATES, TRASH, UNTAGGED } from '../../../../src/search'
import { siteIconCache } from '../../../../src/site-icons'
import { api, type Entry, type Listing } from './api'
import { newEntry } from './editor'
import { entryScreen } from './entry'
import { type IconName, svgIcon } from './icons'
import { app, confirmSheet, copied, iconButton, overlay, screens, sheet, show, snack } from './page'
import { settingsPeek, settingsScreen } from './settings'
import { closing, slide, type Sliding, swipes } from './slides'
import { askForKey, otherKeyScreen, pullToSync, syncButton, syncSheet } from './sync'
import { toolsSheet } from './tools'
import { lock } from './unlock'

// The list of entries, with its drawer of groups and tags, its search, its
// selection and the sheets opened from it.

/** The list, as Keepass2Android lays it out: a toolbar on top, the entries,
 *  the search button floating at the bottom right, the sync status under it. */
export function listScreen(opened: Listing) {
  let listing = opened
  const title = el('h1', {})
  const searchField = el('input', { type: 'search', className: 'field search', value: app.query })
  const status = el('footer', { className: 'status' }, app.syncLine || 'Not synced yet')
  const list = el('ul', { className: 'entries', role: 'listbox', ariaMultiSelectable: 'true', ariaLabel: 'Entries' })
  const inBin = () => search(listing.entries, '', TRASH).length
  const emptyBin = button('Empty the recycle bin…', 'Delete everything in it for good', () =>
    confirmSheet(`Delete ${entriesCount(inBin())} permanently? This cannot be undone: other devices delete them too when they sync.`, 'Delete permanently', () =>
      void api.emptyTrash().then(toListSaying('Deleted permanently'), (e) => snack(String(e)))), 'link danger above-list')
  const starters = button('Add starter templates', 'Credit card, bank account, ID document, email account and Wi-Fi', () =>
    void api.addStarterTemplates().then(toListSaying('Starter templates added'), (e) => snack(String(e))), 'link above-list')
  // Several entries chosen with a long press (not in the trash): a bar of
  // what can be done to them all replaces the toolbar until it is closed.
  const chosen = new Set<string>()
  const chosenTitle = el('h1', {})
  const copyChosen = iconButton('copy', 'Copy from it', () => quickCopy(listing.entries.find((e) => chosen.has(e.id))!))
  const tagChosen = iconButton('tag', 'Add or remove a tag', () => tagSheet([...chosen], listing, changedSeveral))
  const selecting = el('header', { className: 'bar', hidden: true },
    iconButton('close', 'Clear the selection', () => clearSelection()),
    chosenTitle,
    copyChosen,
    tagChosen,
    iconButton('trash', 'Move to the recycle bin', () =>
      confirmSheet(`Move ${entriesCount(chosen.size)} to the recycle bin?`, 'Delete', () =>
        void api.deleteEntries([...chosen]).then(toListSaying('Moved to the recycle bin'), (e) => snack(String(e))))))
  const toggle = (id: string) => {
    if (!chosen.delete(id)) chosen.add(id)
    marked()
  }
  const clearSelection = () => {
    chosen.clear()
    marked()
  }
  /** The bar and the rows as the selection is. */
  const marked = () => {
    const on = chosen.size > 0
    chosenTitle.textContent = `${chosen.size} selected`
    copyChosen.hidden = chosen.size !== 1
    // Templates are not tagged from here, as on Windows: only edited or deleted.
    tagChosen.hidden = sameFilter(app.filter, TEMPLATES)
    selecting.hidden = !on
    toolbar.hidden = searching.hidden = on
    findButton.hidden = on || searching.isConnected
    add.hidden = on
    // Back clears it, after whatever was open over it (a sheet) is closed.
    if (!on) screens.drop(clearSelection)
    else screens.push(clearSelection)
    for (const item of list.querySelectorAll<HTMLElement>('li[data-id]')) {
      const picked = chosen.has(item.dataset.id!)
      item.classList.toggle('selected', picked)
      item.setAttribute('aria-selected', String(picked))
    }
  }
  const changedSeveral = (fresh: Listing, message: string) => {
    listing = fresh
    chosen.clear()
    fill()
    snack(message)
  }
  const pick = (entry: Entry) => {
    if (chosen.size) return toggle(entry.id)
    app.leaveList(scroll.scrollTop, entry.id)
    void entryScreen(entry.id, listing)
  }
  const press = (entry: Entry) => (sameFilter(app.filter, TRASH) ? quickCopy(entry) : toggle(entry.id))
  const fill = () => {
    title.textContent = filterLabel(app.filter)
    searchField.placeholder = `Search ${filterLabel(app.filter)}`
    emptyBin.hidden = !sameFilter(app.filter, TRASH) || !inBin()
    starters.hidden = !sameFilter(app.filter, TEMPLATES) || listing.entries.some((e) => e.kind === 'template')
    const newLabel = sameFilter(app.filter, TEMPLATES) ? 'New template' : 'New entry'
    add.title = newLabel
    add.setAttribute('aria-label', newLabel)
    const shown = search(listing.entries, app.query, app.filter)
    // What is no longer shown is no longer chosen.
    for (const id of [...chosen]) if (!shown.some((e) => e.id === id)) chosen.delete(id)
    list.replaceChildren(...shown.map((entry) => row(entry, listing, pick, press)))
    if (shown.length === 0) list.append(el('li', { className: 'muted empty' }, 'Nothing here.'))
    marked()
  }
  searchField.addEventListener('input', () => {
    app.query = searchField.value
    fill()
  })
  status.addEventListener('click', syncSheet)
  const scroll = el('div', { className: 'scroll' }, emptyBin, starters, list)
  pullToSync(scroll, () => (status.textContent = 'Syncing…'))
  // Not while entries are chosen (the bar has the list then), nor when turned off.
  swipes(scroll, () => !chosen.size && app.swipesOn(), {
    right: () => drawer(listing, fill, true),
    left: () => settingsPeek(() => listScreen(listing)),
  })
  // Another device changed the key: here until it is entered (as Windows' Enter key…).
  const newKey = iconButton('keyRound', 'Enter the new key', () => void otherKeyScreen())
  newKey.hidden = !app.otherKey
  const toolbar = el('header', { className: 'bar' },
    iconButton('menu', 'Groups and tags', () => drawer(listing, fill)),
    title,
    newKey,
    syncButton(() => (status.textContent = 'Syncing…')),
    iconButton('tools', 'Tools', () => toolsSheet(listing)),
    iconButton('settings', 'Settings', () => void settingsScreen(() => listScreen(listing))),
    iconButton('lock', 'Lock', () => void lock()))
  const closeSearch = () => {
    searching.replaceWith(toolbar)
    findButton.hidden = false
    screens.drop(closeSearch)
    app.query = ''
    searchField.value = ''
    fill()
  }
  const searching = el('header', { className: 'bar' }, iconButton('back', 'Close the search', closeSearch), searchField)
  const findButton = iconButton('search', 'Search', () => {
    toolbar.replaceWith(searching)
    findButton.hidden = true
    screens.push(closeSearch)
    searchField.focus()
  }, 'fab')
  const add = iconButton('plus', 'New entry', () => newEntry(listing), 'fab')
  show([selecting, app.query ? searching : toolbar, scroll, el('div', { className: 'fabs' }, findButton, add), status], undefined, 'list-screen')
  if (app.query) {
    findButton.hidden = true
    screens.push(closeSearch)
  }
  fill()
  // Back from an entry opened here: the list where it was, with that entry in
  // view (after an edit it may have moved).
  const place = app.takeListPlace()
  if (place) {
    scroll.scrollTop = place.top
    list.querySelector(`li[data-id="${CSS.escape(place.id)}"]`)?.scrollIntoView({ block: 'nearest' })
  }
  screens.onSynced((synced) => {
    status.textContent = synced.text
    status.classList.toggle('problem', synced.problem)
    newKey.hidden = !synced.otherKey
    if (synced.changed) void api.listing().then((fresh) => ((listing = fresh), fill()))
    askForKey()
  })
  askForKey()
}

const entriesCount = (n: number) => `${n} ${n === 1 ? 'entry' : 'entries'}`

/** After a change made from an entry: the list again, and what happened. */
export const toListSaying = (message: string) => (fresh: Listing) => (listScreen(fresh), snack(message))

function filterLabel(f: Filter) {
  if (f.kind === 'tag') return f.tag
  if (f.kind === 'untagged') return 'Untagged'
  return GROUPS.find((g) => g.group === f.group)!.label
}

/** An entry in the list: a tap does `tap`, a long press `press`. */
function row(entry: Entry, listing: Listing, tap: (entry: Entry) => void, press: (entry: Entry) => void): HTMLLIElement {
  const item = el('li', { tabIndex: 0, role: 'option' }, icon(entry, listing), el('span', {}, el('b', {}, titleOf(entry)), el('small', {}, entry.username)))
  item.dataset.id = entry.id
  item.addEventListener('click', () => tap(entry))
  item.addEventListener('contextmenu', (e) => {
    e.preventDefault()
    press(entry)
  })
  return item
}

/** Adds a tag to entries or takes one off: the tag typed or picked among the
 *  database's (to add) or theirs (to take off). */
function tagSheet(ids: string[], listing: Listing, done: (fresh: Listing, message: string) => void) {
  const theirs = [...new Set(listing.entries.filter((e) => ids.includes(e.id)).flatMap((e) => e.tags))].filter((t) => t !== FAVORITE).sort((a, b) => a.localeCompare(b))
  const known = tagCounts(listing.entries).map(([tag]) => tag)
  const change = (tag: string, on: boolean, close: () => void) => {
    close()
    void api.setTag(ids, tag, on).then((fresh) => done(fresh, `${on ? 'Tagged' : 'Untagged'} ${entriesCount(ids.length)}: #${tag.trim()}`), (e) => snack(String(e)))
  }
  sheet((close) => {
    const name = input('', { className: 'field', ariaLabel: 'Tag', placeholder: 'Tag to add' })
    const offered = el('div', { className: 'tag-offers' })
    // The first dozen that match what is typed: enough to pick from without scrolling the sheet.
    const offer = () =>
      offered.replaceChildren(...known.filter((t) => t.toLowerCase().includes(name.value.trim().toLowerCase())).slice(0, 12)
        .map((t) => button(`#${t}`, `Add the tag ${t}`, () => change(t, true, close), 'tag-offer')))
    name.addEventListener('input', offer)
    offer()
    const addTyped = button('Add', 'Add the tag typed', () => name.value.trim() && change(name.value, true, close), 'primary')
    enterPresses(addTyped, name)
    return [
      el('b', {}, `Tag ${entriesCount(ids.length)}`),
      name, offered, addTyped,
      ...(theirs.length ? [el('b', {}, 'Take off'), ...theirs.map((t) => button(`#${t}`, `Take the tag ${t} off`, () => change(t, false, close), 'item danger'))] : []),
    ]
  })
}

/** Site icons by host; one that arrives replaces the key in the images waiting for it. */
const siteIcons = siteIconCache(api.icon, (host, src) => {
  document.querySelectorAll<HTMLImageElement>('img.icon[data-host]').forEach((img) => {
    if (img.dataset.host !== host) return
    img.src = src
    img.classList.add('site')
  })
})
void listen<string>('icon-ready', (e) => siteIcons.refresh(e.payload))

/** An entry's own image, else the drawing chosen for it, else its site's icon
 *  (when it arrives), else the key: as on Windows. Images drawn for light
 *  pages (the entry's own, a site's) are marked `site` for their light tile. */
export function icon(entry: Entry, listing: Listing) {
  const custom = entry.customIcon && listing.customIcons[entry.customIcon]
  if (custom) return el('img', { className: 'icon site', src: custom, alt: '' })
  if (entry.icon !== null) return el('img', { className: 'icon', src: glyphIcon(entry.icon), alt: '' })
  const site = entry.host && siteIcons.get(entry.host)
  const img = el('img', { className: site ? 'icon site' : 'icon', src: site || DEFAULT_ICON, alt: '' })
  if (entry.host) img.dataset.host = entry.host
  return img
}

/** A long press: copy without opening the entry. */
function quickCopy(entry: Entry) {
  const choices: [string, () => Promise<number>][] = [
    ['Copy user name', () => api.copyField(entry.id, 'UserName')],
    ['Copy password', () => api.copyField(entry.id, 'Password')],
  ]
  if (entry.otp) choices.push(['Copy TOTP code', () => api.copyTotp(entry.id)])
  sheet((close) => [el('b', {}, titleOf(entry)), ...choices.map(([label, copy]) => button(label, label, () => void copied(copy()).then(close), 'item'))])
}

/** The drawer; it slides in, unless a swipe brings it in step with the finger (`dragged`). */
function drawer(listing: Listing, changed: () => void, dragged = false): Sliding {
  const panel = el('nav', { className: 'drawer' })
  const shade = el('div', { className: 'shade' }, panel)
  const close = overlay(shade)
  const item = (f: Filter, label: string, count: number, icon?: IconName) => {
    const choose = () => {
      app.filter = f
      close()
      changed()
    }
    const name = el('span', {}, ...(icon ? [svgIcon(icon)] : []), label)
    const b = el('button', { type: 'button', onclick: choose }, name, el('small', {}, String(count)))
    b.classList.toggle('tag', f.kind === 'tag')
    b.classList.toggle('chosen', sameFilter(f, app.filter))
    return b
  }
  const entries = listing.entries
  panel.append(
    el('b', { className: 'title' }, app.database?.title ?? ''),
    el('small', {}, [app.database?.syncedWith, app.syncLine].filter(Boolean).join(' · ')),
    el('h2', {}, 'Groups'),
    ...GROUPS.map(({ group, label }) => {
      const f: Filter = { kind: 'group', group }
      return item(f, label, search(entries, '', f).length, GROUP_ICONS[group])
    }),
    el('h2', {}, 'Tags'),
    item(UNTAGGED, 'Untagged', search(entries, '', UNTAGGED).length),
    ...tagCounts(entries).map(([tag, count]) => item({ kind: 'tag', tag }, tag, count)),
    el('p', { className: 'version' }, `Version ${app.settings?.version ?? ''}`),
  )
  const sliding = slide(shade, panel, -1, close)
  if (!dragged) sliding.release(true)
  // A swipe back to the left takes it away with the finger.
  swipes(shade, app.swipesOn, { left: () => closing(sliding) })
  return sliding
}
