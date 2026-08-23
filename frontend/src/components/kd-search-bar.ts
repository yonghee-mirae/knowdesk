// Search input + content/filename mode toggle (`docs/12_UI_Spec.md` C1).
// Dumb component: emits raw events on every keystroke/click, `main.ts` owns
// debouncing and the actual search call.
//
// The theme-toggle and settings-folder icon buttons that used to live here
// were removed - theme is now a `settings.json` setting applied on load
// (`core/theme.ts`), and "설정" is reachable from the tray menu, so there's
// no separate in-window button needed for either.

import type { SearchMode } from '../types';
import { MOD_KEY } from '../core/platform';

export class KdSearchBar extends HTMLElement {
  private inputEl_: HTMLInputElement;
  private contentBtn: HTMLButtonElement;
  private filenameBtn: HTMLButtonElement;

  constructor() {
    super();
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <style>
        .bar {
          display: flex;
          align-items: center;
          gap: 10px;
          padding: 14px 16px;
          border-bottom: 1px solid var(--border);
        }
        svg { flex: none; color: var(--ink-faint); }
        input {
          flex: 1;
          min-width: 0;
          border: none;
          outline: none;
          background: transparent;
          color: var(--ink);
          font-family: inherit;
          font-size: 17px;
          font-weight: 500;
        }
        input::placeholder { color: var(--ink-faint); font-weight: 400; }
        .mode-toggle {
          flex: none;
          display: flex;
          background: var(--surface-2);
          border: 1px solid var(--border);
          padding: 2px;
          gap: 2px;
        }
        .mode-toggle button {
          border: none;
          background: transparent;
          color: var(--ink-muted);
          font-family: inherit;
          font-size: 12.5px;
          font-weight: 600;
          padding: 5px 10px;
          cursor: pointer;
        }
        .mode-toggle button.active {
          background: var(--surface);
          color: var(--accent);
          box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
        }
        .mode-toggle button:focus-visible,
        input:focus-visible {
          outline: 2px solid var(--accent);
          outline-offset: 2px;
        }
      </style>
      <div class="bar">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <circle cx="11" cy="11" r="7"></circle>
          <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
        </svg>
        <input id="query" type="text" placeholder="검색어를 입력하세요." autocomplete="off" spellcheck="false" />
        <div class="mode-toggle" role="tablist" aria-label="검색 모드">
          <button id="mode-content" class="active" role="tab" aria-selected="true" title="${MOD_KEY}+1">내용</button>
          <button id="mode-filename" role="tab" aria-selected="false" title="${MOD_KEY}+2">파일명</button>
        </div>
      </div>
    `;
    this.inputEl_ = root.querySelector('#query')!;
    this.contentBtn = root.querySelector('#mode-content')!;
    this.filenameBtn = root.querySelector('#mode-filename')!;

    this.inputEl_.addEventListener('input', () => {
      this.dispatchEvent(new CustomEvent<string>('kd-query-input', { detail: this.inputEl_.value }));
    });
    this.contentBtn.addEventListener('click', () => this.setMode('content', true));
    this.filenameBtn.addEventListener('click', () => this.setMode('filename', true));
  }

  /** The real `<input>` inside the shadow root - used by `main.ts`'s focus guard
   * and initial-focus call, since `event.target` at the document level would
   * otherwise just be this host element. */
  get inputEl(): HTMLInputElement {
    return this.inputEl_;
  }

  get value(): string {
    return this.inputEl_.value;
  }

  focusInput(): void {
    this.inputEl_.focus();
  }

  /** Empties the query input without dispatching `kd-query-input` - the
   * caller (`main.ts`'s Esc handler) drives the resulting empty-state UI
   * itself rather than going through the debounced search path. */
  clear(): void {
    this.inputEl_.value = '';
  }

  setMode(mode: SearchMode, emit = false): void {
    const isContent = mode === 'content';
    this.contentBtn.classList.toggle('active', isContent);
    this.filenameBtn.classList.toggle('active', !isContent);
    this.contentBtn.setAttribute('aria-selected', String(isContent));
    this.filenameBtn.setAttribute('aria-selected', String(!isContent));
    if (emit) {
      this.dispatchEvent(new CustomEvent<SearchMode>('kd-mode-change', { detail: mode }));
    }
  }
}

customElements.define('kd-search-bar', KdSearchBar);
