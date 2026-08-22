// Thin wrapper around the Tauri IPC commands (`src-tauri/src/lib.rs`) - keeps
// `invoke()` calls out of the components/orchestration code.

import { invoke } from '@tauri-apps/api/core';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import type { SearchHit, SearchMode } from '../types';
import type { Theme } from '../core/theme';

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

/** Reads the `theme` field from `settings.json` (`core::config::Theme`). */
export async function getTheme(): Promise<Theme> {
  return invoke<Theme>('get_theme');
}
