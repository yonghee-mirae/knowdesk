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

export async function openSettingsWindow(): Promise<void> {
  await invoke('open_settings_window');
}

export async function getWatchedFolders(): Promise<string[]> {
  return invoke<string[]>('get_watched_folders');
}

/** Returns the updated folder list. */
export async function addWatchedFolder(path: string): Promise<string[]> {
  return invoke<string[]>('add_watched_folder', { path });
}

/** Returns the updated folder list. */
export async function removeWatchedFolder(path: string): Promise<string[]> {
  return invoke<string[]>('remove_watched_folder', { path });
}

/** Opens a native folder-picker dialog. `null` if the user cancels. */
export async function openFolderPicker(): Promise<string | null> {
  return invoke<string | null>('open_folder_picker');
}
