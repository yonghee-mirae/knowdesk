// Preview pane (`docs/12_UI_Spec.md` C2) - shows the selected result's
// metadata and a highlighted body snippet. Index tier (본문 색인/메타 색인) is
// shown as a text value in the metadata table - META-tier hits show "Meta"
// there instead of a snippet, since their body was never extracted
// (`docs/04_Data_Model.md`).

import type { SearchHit } from '../types';
import { renderSnippet, escapeHtml } from '../core/snippet';
import { formatLocalDateTime } from '../core/datetime';

export class KdPreview extends HTMLElement {
  private bodyEl: HTMLDivElement;
  /** The hit currently shown, so a body-preview fetch that resolves after
   * the user has already moved on (`showBodyPreview`) can tell it's stale. */
  private currentPath: string | null = null;

  constructor() {
    super();
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <style>
        :host { display: block; overflow-y: auto; }
        :host([hidden]) { display: none; }
        .body { padding: 20px 22px; display: flex; flex-direction: column; gap: 14px; }
        .p-title {
          font-size: 16px;
          font-weight: 700;
          line-height: 1.4;
        }
        .p-title-text { text-wrap: balance; }
        /* Path, modified date, and index tier, grouped into one metadata box. */
        .p-meta {
          font-family: var(--font-mono);
          font-size: 11.5px;
          border: 1px solid var(--border);
        }
        .p-meta-row {
          background: var(--surface-2);
          padding: 8px 10px;
        }
        .p-meta-row + .p-meta-row { border-top: 1px solid var(--border); }
        .p-meta-path { color: var(--ink-muted); word-break: break-all; }
        .p-meta-field-row { display: flex; gap: 8px; }
        .p-meta-label { flex: none; width: 40px; color: var(--ink-faint); }
        .p-meta-value { color: var(--ink); }
        .p-snippet {
          font-size: 13.5px;
          line-height: 1.75;
          color: var(--ink);
          padding-top: 14px;
          border-top: 1px solid var(--border);
        }
        mark {
          background: var(--mark-bg);
          color: var(--mark-ink);
          border-radius: 3px;
          padding: 0 2px;
          font-weight: 600;
        }
      </style>
      <div class="body"></div>
    `;
    this.bodyEl = root.querySelector('.body')!;
  }

  /** Nothing to show: query is empty (`kd-syntax-help` covers that case and
   * this pane is hidden entirely) or the search found zero hits. */
  clear(): void {
    this.currentPath = null;
    this.bodyEl.innerHTML = '';
  }

  /** Renders everything known synchronously. If there's no snippet (a
   * filter-only query, or filename mode - neither has a keyword to build one
   * around) and the document actually has a body (FULL tier), the snippet
   * area is left empty for `main.ts` to fill in via `showBodyPreview()` once
   * it fetches the document's opening text (`docs/12_UI_Spec.md` C2) - a
   * separate, on-demand call so the result list's every-row search response
   * doesn't need to carry it. */
  showHit(hit: SearchHit): void {
    this.currentPath = hit.path;
    const tierText = hit.indexTier === 'FULL' ? 'Full' : 'Meta';

    let html = '';
    html += `<div class="p-title">
      <span class="p-title-text">${escapeHtml(hit.filename)}</span>
    </div>`;
    html += `<div class="p-meta">
      <div class="p-meta-row p-meta-path">${escapeHtml(hit.path)}</div>
      <div class="p-meta-row p-meta-field-row">
        <span class="p-meta-label">수정일</span>
        <span class="p-meta-value">${hit.modifiedAt ? escapeHtml(formatLocalDateTime(hit.modifiedAt)) : '-'}</span>
      </div>
      <div class="p-meta-row p-meta-field-row">
        <span class="p-meta-label">색인</span>
        <span class="p-meta-value">${tierText}</span>
      </div>
    </div>`;

    if (hit.snippet !== null) {
      html += `<div class="p-snippet">${renderSnippet(hit.snippet)}</div>`;
    }

    this.bodyEl.innerHTML = html;
  }

  /** Fills in the document's opening text fetched for `path` - a no-op if
   * the preview has since moved on to a different hit (fast arrow-key
   * navigation outrunning the fetch), the fetch found nothing to show, or a
   * snippet is already showing (re-selecting the same still-pending hit
   * twice must not append a second copy once both fetches land). */
  showBodyPreview(path: string, text: string | null): void {
    if (path !== this.currentPath || text === null) return;
    if (this.bodyEl.querySelector('.p-snippet')) return;
    const div = document.createElement('div');
    div.className = 'p-snippet';
    div.textContent = text; // Plain text - no `>>...<<` markers to render as marks.
    this.bodyEl.appendChild(div);
  }
}

customElements.define('kd-preview', KdPreview);
