// Color theme, read from `settings.json`'s `theme` field (`core::config::Theme`
// on the backend) rather than a toggle button - there is none anymore. `system`
// (the default) means no explicit override; `tokens.css`'s `prefers-color-scheme`
// media query then decides.

import { getTheme } from '../platform/backend';

export type Theme = 'light' | 'dark' | 'system';

/** Applies (or clears, for `'system'`) an explicit override on `<html
 * data-theme>` - `tokens.css`'s `:root[data-theme]` rules pick this up over
 * the OS setting. */
export function applyTheme(theme: Theme): void {
  if (theme === 'system') {
    delete document.documentElement.dataset['theme'];
  } else {
    document.documentElement.dataset['theme'] = theme;
  }
}

/** Reads `theme` from `settings.json` (via the backend) and applies it.
 * Called once at page load, and again every time the search window regains
 * focus - the window is created once and just shown/hidden for the rest of
 * the app's life, so re-checking on focus is what makes a hand-edited theme
 * setting actually take effect without needing a push-based update. Falls
 * back to `system` if the read fails for any reason. */
export async function loadAndApplyTheme(): Promise<void> {
  let theme: Theme = 'system';
  try {
    theme = await getTheme();
  } catch {
    // Best-effort - an unreadable/corrupt settings.json shouldn't block the
    // rest of the window from working, just fall back to the OS setting.
  }
  applyTheme(theme);
}
