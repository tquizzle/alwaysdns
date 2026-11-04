// Per-viewer conveniences only. Every access is guarded because storage can be
// blocked, full, or unavailable (private windows, sandboxed previews).

export function readStorage(key: string, fallback: string): string {
  try {
    return window.localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

export function writeStorage(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Ignore: the preference simply does not persist.
  }
}
