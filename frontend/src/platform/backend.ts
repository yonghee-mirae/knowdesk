// Thin wrapper around the Tauri IPC commands (`src-tauri/src/lib.rs`) - keeps
// `invoke()` calls out of the components/orchestration code.

import { invoke } from '@tauri-apps/api/core';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import type { SearchHit, SearchMode } from '../types';

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

/** "설정" action: there's no in-app Settings window - this opens the folder
 * containing `settings.json` in the OS file manager, so editing that file
 * directly is the whole UI for now. */
export async function openSettingsFolder(): Promise<void> {
  await invoke('open_settings_folder');
}
