// Shared between the result-row badge and the preview panel's index-tier
// icon - one place to keep icon/label per match kind in sync
// (`docs/12_UI_Spec.md` C1/C2).

export type BadgeKind = 'exact' | 'morph' | 'meta' | 'full';

/** `icon` is raw HTML (an emoji character, or an inline SVG using
 * `stroke="currentColor"` so it recolors with the theme via CSS `color`) -
 * whoever renders it inserts it directly, not escaped. */
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
  //
  // Monochrome line icons (not colored emoji) so they read as UI chrome, not
  // decoration, and recolor automatically for light/dark via `currentColor`.
  // Path data is Feather's (feathericons.com, MIT) "lock"/"file-text" glyphs
  // as-is, not hand-drawn - both keep the same 2px top/bottom padding within
  // their 24x24 viewBox, which matters for lining up with adjacent text: an
  // earlier hand-drawn lock had asymmetric padding (7px top, 4px bottom) that
  // visibly floated above the text baseline no matter how the icon itself
  // was aligned, since the drawn shape sat well above the icon's own box
  // bottom to begin with.
  meta: {
    icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"></rect><path d="M7 11V7a5 5 0 0 1 10 0v4"></path></svg>',
    label: '본문 미색인',
  },
  full: {
    icon: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path><polyline points="14 2 14 8 20 8"></polyline><line x1="16" y1="13" x2="8" y2="13"></line><line x1="16" y1="17" x2="8" y2="17"></line></svg>',
    label: '본문 색인',
  },
};
