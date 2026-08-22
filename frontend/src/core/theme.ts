// Dark-mode toggle (approved prototype). The OS setting (`prefers-color-scheme`,
// via `tokens.css`) is the default; this only persists an explicit override.

const STORAGE_KEY = 'knowdesk-theme';

export type Theme = 'light' | 'dark';

function readStorage(): Theme | null {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return stored === 'light' || stored === 'dark' ? stored : null;
  } catch {
    return null;
  }
}

function writeStorage(theme: Theme | null): void {
  try {
    if (theme) localStorage.setItem(STORAGE_KEY, theme);
    else localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Best-effort - a per-viewer convenience, not something that must persist reliably.
  }
}

/** Applies (or clears) an explicit override on `<html data-theme>` - `tokens.css`'s
 * `:root[data-theme]` rules pick this up over the OS setting. */
export function applyTheme(theme: Theme | null): void {
  if (theme) {
    document.documentElement.dataset['theme'] = theme;
  } else {
    delete document.documentElement.dataset['theme'];
  }
}

/** Loads and applies whatever override (if any) was saved from a previous session. */
export function initTheme(): void {
  applyTheme(readStorage());
}

/** The theme actually in effect right now - the stored override, or the OS
 * setting if there isn't one. */
export function effectiveTheme(): Theme {
  return readStorage() ?? (window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
}

/** Flips the current effective theme and persists it as an explicit override. */
export function toggleTheme(): Theme {
  const next: Theme = effectiveTheme() === 'dark' ? 'light' : 'dark';
  writeStorage(next);
  applyTheme(next);
  return next;
}
