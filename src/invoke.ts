import { invoke as tauriInvoke, type InvokeArgs } from '@tauri-apps/api/core'

/** A failure a caller acts on, as the backend names it (`pswm_core::error::Code`). */
export type ErrorCode = 'cancelled'

/** A command's error that says which failure it is, for a caller that acts on
 *  it. As text it is its message, so it is shown like the plain-text ones. */
export class CommandError {
  constructor(readonly code: ErrorCode | null, readonly message: string) {}

  toString(): string {
    return this.message
  }
}

/** What a failed command threw, as the UI takes it: the backend's
 *  `{ code, message }` as a `CommandError`, anything else (plain text) as it is. */
export function thrown(error: unknown): unknown {
  if (typeof error !== 'object' || error === null) return error
  const { code, message } = error as { code?: ErrorCode | null; message?: unknown }
  return typeof message === 'string' ? new CommandError(code ?? null, message) : error
}

/** The user closed the command's prompt (Windows Hello's, the fingerprint's):
 *  nothing went wrong, and there is nothing to show. */
export const isCancelled = (error: unknown): boolean => error instanceof CommandError && error.code === 'cancelled'

/** Calls a command of the backend; its error is thrown as `thrown` gives it. */
export const invoke = <T>(command: string, args?: InvokeArgs): Promise<T> =>
  tauriInvoke<T>(command, args).catch((error: unknown) => {
    throw thrown(error)
  })
