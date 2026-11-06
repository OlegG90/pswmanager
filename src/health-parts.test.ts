import { describe, expect, it } from 'vitest'
import { shownGroups } from './health-parts'

const finding = (id: string) => ({ id, title: id, detail: '' })
const health = { reused: [finding('a'), finding('b')], weak: [], old: [finding('c')] }
const groups = (only: Parameters<typeof shownGroups>[1]) => shownGroups(health, only).map(({ group }) => group)

describe('the lists password health shows', () => {
  it('shows every list with findings when no figure is chosen', () => {
    expect(groups(null)).toEqual(['reused', 'old'])
  })

  it('shows the chosen figure’s list alone', () => {
    expect(groups('old')).toEqual(['old'])
  })

  it('shows every list when the chosen one has no findings', () => {
    expect(groups('weak')).toEqual(['reused', 'old'])
  })
})
