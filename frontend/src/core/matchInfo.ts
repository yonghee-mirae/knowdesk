// Shared between the result-row badge and the preview panel's "본문 미색인"
// notice - one place to keep icon/label per match kind in sync
// (`docs/12_UI_Spec.md` C1/C2).

export type BadgeKind = 'exact' | 'morph' | 'meta';

export const MATCH_INFO: Record<BadgeKind, { icon: string; label: string }> = {
  exact: { icon: '🎯', label: '정확 일치' },
  morph: { icon: '🌱', label: '형태소 분석' },
  // META tier isn't always DRM/security - it also covers CORRUPT/ENCRYPTED/
  // PARSE_FAIL/EMPTY_TEXT (`docs/04_Data_Model.md`), e.g. samples.db's own
  // 손상.pdf (a corrupted PDF, nothing to do with security). Don't claim a
  // reason the UI doesn't actually know.
  meta: { icon: '🔒', label: '본문 미색인' },
};
