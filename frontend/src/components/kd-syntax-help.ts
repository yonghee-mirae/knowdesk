// Search-syntax help (`docs/12_UI_Spec.md` C1) - shown in place of the result
// list while the query is empty. Purely presentational: `main.ts` calls
// `render(mode)` whenever the mode changes or the query becomes/stops being
// empty.

import { escapeHtml } from '../core/snippet';
import { MODE_DESC, CONTENT_SEARCH_HELP, FILENAME_SEARCH_HELP, type HelpItem, type HelpPart } from '../core/searchHelp';
import type { SearchMode } from '../types';

function renderParts(parts: HelpPart[]): string {
  return parts
    .map((p) => {
      const cls = p.r === 'k' ? 'sh-kw' : p.r === 'v' ? 'sh-val' : '';
      return cls ? `<span class="${cls}">${escapeHtml(p.t)}</span>` : escapeHtml(p.t);
    })
    .join('');
}

export class KdSyntaxHelp extends HTMLElement {
  private bodyEl: HTMLDivElement;

  constructor() {
    super();
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <style>
        :host { display: block; overflow-y: auto; padding: 4px 6px 8px; }
        :host([hidden]) { display: none; }
        .sh-mode-desc {
          font-size: 12px;
          line-height: 1.5;
          color: var(--ink-muted);
          padding: 8px 4px 12px;
          border-bottom: 1px solid var(--border);
          margin-bottom: 4px;
        }
        .sh-group-label {
          padding: 14px 4px 6px;
          font-size: 12px;
          font-weight: 700;
          letter-spacing: 0.05em;
          text-transform: uppercase;
          color: var(--ink);
        }
        .sh-row { display: flex; align-items: baseline; gap: 10px; padding: 6px 4px; }
        .sh-example {
          flex: none;
          width: 172px;
          font-family: var(--font-mono);
          font-size: 12px;
          overflow: hidden;
          text-overflow: ellipsis;
          white-space: nowrap;
        }
        .sh-kw { font-weight: 700; color: var(--accent); }
        .sh-val { font-weight: 400; color: var(--ink-faint); }
        .sh-desc { font-size: 12px; color: var(--ink-muted); line-height: 1.5; }
      </style>
      <div class="body"></div>
    `;
    this.bodyEl = root.querySelector('.body')!;
  }

  render(mode: SearchMode): void {
    const items = mode === 'filename' ? FILENAME_SEARCH_HELP : CONTENT_SEARCH_HELP;
    const groups: string[] = [];
    for (const item of items) {
      if (!groups.includes(item.group)) groups.push(item.group);
    }

    let html = `<div class="sh-mode-desc">${escapeHtml(MODE_DESC[mode])}</div>`;
    for (const group of groups) {
      html += `<div class="sh-group-label">${escapeHtml(group)}</div>`;
      html += items
        .filter((item): item is HelpItem => item.group === group)
        .map(
          (item) =>
            `<div class="sh-row"><span class="sh-example">${renderParts(item.parts)}</span><span class="sh-desc">${escapeHtml(item.desc)}</span></div>`,
        )
        .join('');
    }
    this.bodyEl.innerHTML = html;
  }
}

customElements.define('kd-syntax-help', KdSyntaxHelp);
