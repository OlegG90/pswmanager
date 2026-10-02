/**
 * Sites' icons as `data:` URLs, asked for once per host. `fetch` asks the
 * backend; `shown` puts an icon that arrived where it is waited for (each app
 * has its own page). Shared by the Windows and the Android page.
 */
export function siteIconCache(fetch: (host: string) => Promise<string | null>, shown: (host: string, src: string) => void) {
  /** `null` while asked for, or when there is none. */
  const icons = new Map<string, string | null>()
  const load = async (host: string) => {
    icons.set(host, null)
    const src = await fetch(host).catch(() => null)
    if (!src) return
    icons.set(host, src)
    shown(host, src)
  }
  return {
    /** The icon when it is here; asked for, once, otherwise. */
    get(host: string): string | null {
      if (!icons.has(host)) void load(host)
      return icons.get(host) ?? null
    },
    /** The backend has a newer one (event `icon-ready`). */
    refresh(host: string) {
      icons.delete(host)
      void load(host)
    },
  }
}
