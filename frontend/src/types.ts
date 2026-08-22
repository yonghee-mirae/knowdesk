// Matches `src-tauri/src/lib.rs`'s `SearchHitDto`.
export interface SearchHit {
  path: string;
  filename: string;
  snippet: string | null;
  matchKind: 'exact' | 'morphological';
  extension: string;
  modifiedAt: string | null;
  /** 'FULL' | 'META' | 'SKIP' (`docs/04_Data_Model.md`) - SKIP never appears in results. */
  indexTier: string;
}

export type SearchMode = 'content' | 'filename';
