import type { Saved } from './api'

/** What the open editor does when the window asks it to. */
interface Open {
  save: () => Promise<void>
  close: () => Promise<void>
  /** Closes without saving or asking: nothing of the edit is kept. */
  discard: () => void
  /** Puts the keyboard back where it was in the form. */
  focus: () => void
  /** The entries another device just changed. */
  changedElsewhere: (ids: string[]) => void
}

/** What an editor needs from the form and the backend. */
export interface EditorRules {
  /** The entry edited; null for a new one. */
  id: string | null
  /** Whether the form (its files included) differs from how it opened. */
  dirty: () => boolean
  /** Saves the form, as the backend answers. */
  store: () => Promise<Saved>
  saved: (saved: Saved) => void
  /** Closed without saving: what the form staged goes, the entry shows as it was. */
  closed: () => void
  focus: () => void
  /** Says something went wrong, in the form. */
  fail: (message: string) => void
}

/** The questions the editor asks, on the window's dialogs. */
export interface EditorQuestions {
  ask: (question: string, yes: string, no: string) => Promise<boolean>
  isAsking: () => boolean
}

export const CHANGED_ELSEWHERE = 'This entry was just changed on another device. Saving keeps your version; the other one goes to the entry\'s history.'

const nothing = async () => {}

/**
 * The window's one editor, from the moment it starts loading to the moment
 * it saves or closes, with the rules on what happens meanwhile: locking (see
 * [clear]) leaves no editor however far it got, a second Save waits for the
 * first, and a question that was answered too late does nothing. Nothing of
 * the page: the form is drawn, and the backend asked, by the caller.
 */
export function editorSession(questions: EditorQuestions) {
  let active: Open | null = null

  return {
    isEditing: (): boolean => active !== null,

    /** The vault locked: whatever editor there was is gone. */
    clear() {
      active = null
    },

    save: (): Promise<void> => active?.save() ?? nothing(),
    close: (): Promise<void> => active?.close() ?? nothing(),

    changedElsewhere(ids: string[]) {
      active?.changedElsewhere(ids)
    },

    /**
     * Something outside the editor was chosen while it is open: asks whether
     * to leave it. Yes closes it without saving, No keeps the user in it.
     * True when there is no editor any more.
     */
    async leave(): Promise<boolean> {
      const open = active
      if (!open) return true
      if (questions.isAsking()) return false
      const leave = await questions.ask('Leave editing? The changes are not saved.', 'Yes', 'No')
      // Saved, cancelled or locked while asking: what was chosen is not done.
      if (active !== open) return false
      if (leave) open.discard()
      else open.focus()
      return leave
    },

    /**
     * Loads what an editor shows; it counts as open meanwhile, so a second
     * Ctrl+E or a click in the list waits. Null when the vault locked
     * meanwhile; when loading fails, there is no editor and the error is thrown.
     */
    async load<T>(load: () => Promise<T>): Promise<T | null> {
      const loading: Open = { save: nothing, close: nothing, discard: () => {}, focus: () => {}, changedElsewhere: () => {} }
      active = loading
      try {
        const loaded = await load()
        return active === loading ? loaded : null
      } catch (e) {
        if (active === loading) active = null
        throw e
      }
    },

    /** The loaded editor is open by `rules`; what its Save and Cancel buttons do. */
    open(rules: EditorRules): { save: () => Promise<void>; close: () => Promise<void> } {
      let saving = false
      const self: Open = {
        async save() {
          if (saving) return
          saving = true
          try {
            const saved = await rules.store()
            // Locking while the save ran closed this editor: the vault is gone.
            if (active !== self) return
            active = null
            rules.saved(saved)
          } catch (e) {
            if (active === self) rules.fail(String(e))
          } finally {
            saving = false
          }
        },
        discard() {
          active = null
          rules.closed()
        },
        async close() {
          if (rules.dirty()) {
            const yes = await questions.ask('Discard the unsaved changes?', 'Discard', 'Keep editing')
            if (!yes || active !== self) return
          }
          self.discard()
        },
        focus: rules.focus,
        // Saving still works: this edit is applied to the file as it is now,
        // wins where both changed the entry, and the other version goes to
        // its history.
        changedElsewhere(ids) {
          if (rules.id && ids.includes(rules.id)) rules.fail(CHANGED_ELSEWHERE)
        },
      }
      active = self
      return { save: self.save, close: self.close }
    },
  }
}
