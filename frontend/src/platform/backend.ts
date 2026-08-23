// Thin wrapper around the Tauri IPC commands (`src-tauri/src/lib.rs`) - keeps
// `invoke()` calls out of the components/orchestration code.

import { invoke } from '@tauri-apps/api/core';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import type { SearchHit, SearchMode } from '../types';
import type { Theme } from '../core/theme';

/** `limit`: `0` means unlimited (`core::search::SearchRequest::limit`'s doc comment). */
export async function search(query: string, mode: SearchMode, limit: number): Promise<SearchHit[]> {
  return invoke<SearchHit[]>('search', { query, mode, limit });
}

export async function openPath(path: string): Promise<void> {
  await invoke('open_path', { path });
}

export async function openParentFolder(path: string): Promise<void> {
  await invoke('open_parent_folder', { path });
}

export async function copyPath(path: string): Promise<void> {
  await writeText(path);
}

/** "Settings" (tray menu, and `Cmd/Ctrl+,` in the search window) - opens
 * `settings.json` itself with the OS default program for that file type. */
export async function openSettingsFile(): Promise<void> {
  await invoke('open_settings_file');
}

/** Reads the `theme` field from `settings.json` (`core::config::Theme`). */
export async function getTheme(): Promise<Theme> {
  return invoke<Theme>('get_theme');
}

/** Reads the `result_limit` field from `settings.json`. `0` means unlimited. */
export async function getResultLimit(): Promise<number> {
  return invoke<number>('get_result_limit');
}

/** Reads the `search_debounce_ms` field from `settings.json`. */
export async function getSearchDebounceMs(): Promise<number> {
  return invoke<number>('get_search_debounce_ms');
}

/** First ~300 characters of `path`'s stored body text - used when a hit has
 * no snippet (a filter-only query, or filename mode). `null` if `path` isn't
 * indexed, or has no stored body at all (a META-tier hit). */
export async function previewBody(path: string): Promise<string | null> {
  return invoke<string | null>('preview_body', { path });
}

export interface IndexProgress {
  done: number;
  total: number;
}

/** How many of the files under a newly-added watched folder have been
 * scanned so far - `null` while idle (nothing currently being scanned).
 * Polled, not pushed (`docs/12_UI_Spec.md` C5, TASK-904). */
export async function getIndexProgress(): Promise<IndexProgress | null> {
  return invoke<IndexProgress | null>('get_index_progress');
}
