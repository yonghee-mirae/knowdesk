// Preview pane (`docs/12_UI_Spec.md` C2) - shows the selected result's
// metadata and a highlighted body snippet. META-tier hits get the
// "본문 미색인" notice instead of a snippet, since their body was never
// extracted (`docs/04_Data_Model.md`).

import type { SearchHit } from '../types';
import { renderSnippet, escapeHtml } from '../core/snippet';
import { MATCH_INFO } from '../core/matchInfo';
import { formatLocalDateTime } from '../core/datetime';

export class KdPreview extends HTMLElement {
  private bodyEl: HTMLDivElement;

  constructor() {
    super();
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <style>
        :host { display: block; overflow-y: auto; }
        :host([hidden]) { display: none; }
        .body { padding: 20px 22px; display: flex; flex-direction: column; gap: 14px; }
        .p-title { font-size: 16px; font-weight: 700; line-height: 1.4; text-wrap: balance; }
        .p-path {
          font-family: var(--font-mono);
          font-size: 11.5px;
          color: var(--ink-muted);
          word-break: break-all;
          background: var(--surface-2);
          border: 1px solid var(--border);
          padding: 8px 10px;
        }
        .p-meta-grid {
          display: flex;
          gap: 24px;
        }
        .p-meta-item .k {
          font-size: 10.5px;
          text-transform: uppercase;
          letter-spacing: 0.06em;
          color: var(--ink-faint);
        }
        .p-meta-item .v {
          font-family: var(--font-mono);
          font-size: 12.5px;
          color: var(--ink);
          margin-top: 2px;
          white-space: nowrap;
        }
        .p-snippet {
          font-size: 13.5px;
          line-height: 1.75;
          color: var(--ink);
          padding-top: 14px;
          border-top: 1px solid var(--border);
        }
        .p-meta-notice { font-size: 13px; color: var(--tier-meta); }
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
    this.bodyEl.innerHTML = '';
  }

  showHit(hit: SearchHit): void {
    const metaItem = (k: string, v: string): string =>
      `<div class="p-meta-item"><div class="k">${escapeHtml(k)}</div><div class="v">${escapeHtml(v)}</div></div>`;

    let html = '';
    html += `<div class="p-title">${escapeHtml(hit.filename)}</div>`;
    html += `<div class="p-path">${escapeHtml(hit.path)}</div>`;
    html += '<div class="p-meta-grid">';
    html += metaItem('색인 계층', hit.indexTier === 'FULL' ? '본문 색인' : '메타 색인');
    html += metaItem('수정일', hit.modifiedAt ? formatLocalDateTime(hit.modifiedAt) : '-');
    html += '</div>';

    if (hit.indexTier === 'META') {
      html += `<div class="p-meta-notice">${MATCH_INFO.meta.icon} ${escapeHtml(MATCH_INFO.meta.label)}</div>`;
    } else if (hit.snippet !== null) {
      html += `<div class="p-snippet">${renderSnippet(hit.snippet)}</div>`;
    }

    this.bodyEl.innerHTML = html;
  }
}

customElements.define('kd-preview', KdPreview);
