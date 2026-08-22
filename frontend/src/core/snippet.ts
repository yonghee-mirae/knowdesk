// The backend delimits highlighted spans in a snippet with '>>'/'<<'
// (`core/src/search/service.rs`'s `build_snippet`/`widen_highlights`, and FTS5's
// own `snippet()` call in `core/src/db/search_repo.rs`). This renders that into
// escaped HTML with real `<mark>` tags.

export function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

export function renderSnippet(raw: string): string {
  const parts = raw.split(/(>>|<<)/);
  let html = '';
  let inMark = false;
  for (const part of parts) {
    if (part === '>>') {
      inMark = true;
      continue;
    }
    if (part === '<<') {
      inMark = false;
      continue;
    }
    const escaped = escapeHtml(part);
    html += inMark ? `<mark>${escaped}</mark>` : escaped;
  }
  return html;
}
