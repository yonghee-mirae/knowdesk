// Shared between the result-row badge and the preview panel's "본문 미색인"
// notice - one place to keep icon/label per match kind in sync
// (`docs/12_UI_Spec.md` C1/C2).

export type BadgeKind = 'exact' | 'morph' | 'meta';

export const MATCH_INFO: Record<BadgeKind, { icon: string; label: string }> = {
  exact: { icon: '🎯', label: '정확 일치' },
  morph: { icon: '🌱', label: '형태소 분석' },
  // META tier isn't always DRM/security - it just means content extraction
  // failed for some reason (corrupted file, encrypted/DRM, parse error, ...).
  // The specific reason isn't tracked anywhere (`core::index::pipeline`'s
  // `extract_and_index` lumps every extractor error into META the same way,
  // `04_Data_Model.md`'s change history), so don't claim a reason the UI
  // doesn't actually know. A successful extraction with empty body text
  // (e.g. a textless scanned PDF) is still FULL, not META - only an actual
  // extraction error demotes to META.
  meta: { icon: '🔒', label: '본문 미색인' },
};
