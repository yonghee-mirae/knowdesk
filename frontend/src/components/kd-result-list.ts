// Result list (`docs/12_UI_Spec.md` C1). Purely presentational - `main.ts` owns
// the selected index and calls `render()`/`setSelected()` after every keyboard
// or search update.

import type { SearchHit, SearchMode } from '../types';
import { renderSnippet, escapeHtml } from '../core/snippet';
import { MATCH_INFO } from '../core/matchInfo';

/** Filename mode has no stripe at all - it only ever returns `Exact`
 * (`core/src/search/service.rs`), so every row would show the identical
 * color, conveying nothing. Content mode always uses the backend's real
 * `matchKind` - a filter-only query (e.g. `x:pdf` with no keyword) matches a
 * META-tier document exactly on that filter, so it still gets "정확 일치"
 * like any other exact match (`search_content` only assigns `Morphological`
 * when a keyword was actually Kiwi-expanded). What META-tier changes is
 * whether there's a snippet to show at all (see `render()`'s `snippet`
 * below) - not whether this counts as a match. */
function badgeKindOf(hit: SearchHit, mode: SearchMode): 'exact' | 'morph' | null {
  if (mode === 'filename') return null;
  return hit.matchKind === 'exact' ? 'exact' : 'morph';
}

export class KdResultList extends HTMLElement {
  private listEl: HTMLDivElement;
  private hits: SearchHit[] = [];

  constructor() {
    super();
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <style>
        :host {
          display: block;
          overflow-y: auto;
          padding: 8px;
          border-right: 1px solid var(--border);
        }
        :host([hidden]) { display: none; }
        .empty {
          height: 100%;
          display: flex;
          flex-direction: column;
          align-items: center;
          justify-content: center;
          gap: 6px;
          color: var(--ink-faint);
          text-align: center;
          padding: 24px;
        }
        .empty svg { color: var(--border-strong); }
        .empty .hint { font-size: 13px; }
        .empty .hint-sub { font-size: 12px; color: var(--ink-faint); }
        .row {
          position: relative;
          display: flex;
          flex-direction: column;
          gap: 4px;
          padding: 9px 10px;
          cursor: pointer;
          border: 1px solid transparent;
        }
        .row + .row { margin-top: 2px; }
        .row:hover { background: var(--surface-2); }
        .row.selected { background: var(--accent-wash); border-color: var(--accent); }
        /* Match-kind indicator (docs/12_UI_Spec.md C1) - a left edge stripe
           instead of an inline badge, since a badge's x-position next to a
           variable-length filename lands differently on every row and reads
           as misaligned/cluttered. The stripe always sits at the same spot,
           and the row's title attribute (set in render()) carries the label
           a badge's tooltip used to. Filename mode has no stripe at all (see
           badgeKindOf) since it's always the same value there - only rows
           that get an .exact/.morph class reserve room for it. */
        .row.exact,
        .row.morph {
          padding-left: 16px;
        }
        .row.exact::before,
        .row.morph::before {
          content: '';
          position: absolute;
          left: 4px;
          top: 6px;
          bottom: 6px;
          width: 3px;
          border-radius: 2px;
        }
        .row.exact::before { background: var(--accent); }
        .row.morph::before { background: var(--tier-morph); }
        .filename {
          font-size: 13.5px;
          font-weight: 600;
          color: var(--ink);
          overflow: hidden;
          text-overflow: ellipsis;
          white-space: nowrap;
        }
        .snippet-line {
          font-size: 12px;
          color: var(--ink-muted);
          overflow: hidden;
          text-overflow: ellipsis;
          white-space: nowrap;
          line-height: 1.4;
        }
        mark {
          background: var(--mark-bg);
          color: var(--mark-ink);
          border-radius: 3px;
          padding: 0 2px;
          font-weight: 600;
        }
      </style>
      <div class="list" role="listbox" aria-label="검색 결과 목록"></div>
    `;
    this.listEl = root.querySelector('.list')!;
    this.listEl.addEventListener('click', (e) => {
      const row = (e.target as HTMLElement).closest<HTMLElement>('[data-index]');
      if (!row) return;
      this.dispatchEvent(new CustomEvent<number>('kd-row-click', { detail: Number(row.dataset['index']) }));
    });
  }

  /** `noResults` set means the search ran but found nothing - renders the
   * empty-state hint instead of rows. (The "query is empty" case is handled
   * separately by `kd-syntax-help`, not by this component.) */
  render(hits: SearchHit[], mode: SearchMode, noResults: { hint: string; sub: string } | null): void {
    this.hits = hits;
    if (noResults !== null) {
      this.listEl.innerHTML = `
        <div class="empty">
          <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6">
            <circle cx="11" cy="11" r="7"></circle>
            <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
          </svg>
          <div class="hint">${escapeHtml(noResults.hint)}</div>
          <div class="hint-sub">${escapeHtml(noResults.sub)}</div>
        </div>
      `;
      return;
    }
    this.listEl.innerHTML = hits
      .map((hit, index) => {
        const kind = badgeKindOf(hit, mode);
        // A META-tier hit has no snippet (`hit.snippet` is null) - the row
        // just shows the filename alone; index tier is surfaced in the
        // preview pane instead (`kd-preview.ts`'s title-row icon), not
        // repeated as text here.
        const snippet = hit.snippet !== null ? renderSnippet(hit.snippet) : '';
        const rowClass = kind ? `row ${kind}` : 'row';
        const title = kind ? ` title="${escapeHtml(MATCH_INFO[kind].label)}"` : '';
        return `
          <div class="${rowClass}" data-index="${index}" role="option"${title}>
            <div class="filename">${escapeHtml(hit.filename)}</div>
            ${snippet ? `<div class="snippet-line">${snippet}</div>` : ''}
          </div>
        `;
      })
      .join('');
    this.setSelected(0);
  }

  setSelected(index: number): void {
    const rows = this.listEl.querySelectorAll<HTMLElement>('.row');
    rows.forEach((row, i) => row.classList.toggle('selected', i === index));
    rows[index]?.scrollIntoView({ block: 'nearest' });
  }

  get count(): number {
    return this.hits.length;
  }
}

customElements.define('kd-result-list', KdResultList);
