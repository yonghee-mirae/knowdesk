// Search input + content/filename mode toggle (`docs/12_UI_Spec.md` C1).
// Dumb component: emits raw events on every keystroke/click, `main.ts` owns
// debouncing and the actual search call.

import type { SearchMode } from '../types';
import type { Theme } from '../core/theme';
import { MOD_KEY } from '../core/platform';

export class KdSearchBar extends HTMLElement {
  private inputEl_: HTMLInputElement;
  private contentBtn: HTMLButtonElement;
  private filenameBtn: HTMLButtonElement;
  private themeToggleBtn: HTMLButtonElement;
  private settingsBtn: HTMLButtonElement;
  private sunIcon: SVGElement;
  private moonIcon: SVGElement;

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
          border-radius: 8px;
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
          border-radius: 6px;
          cursor: pointer;
        }
        .mode-toggle button.active {
          background: var(--surface);
          color: var(--accent);
          box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
        }
        .mode-toggle button:focus-visible,
        .icon-btn:focus-visible,
        input:focus-visible {
          outline: 2px solid var(--accent);
          outline-offset: 2px;
        }
        .icon-btn {
          flex: none;
          width: 30px;
          height: 30px;
          display: grid;
          place-items: center;
          border: 1px solid transparent;
          background: transparent;
          color: var(--ink-muted);
          border-radius: 8px;
          cursor: pointer;
        }
        .icon-btn:hover { background: var(--surface-2); color: var(--ink); }
        .icon-btn:disabled { cursor: default; opacity: 0.4; }
        .icon-btn:disabled:hover { background: transparent; color: var(--ink-muted); }
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
        <button class="icon-btn" id="theme-toggle" title="테마 전환" aria-label="테마 전환">
          <svg id="icon-sun" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"></circle><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"></path></svg>
          <svg id="icon-moon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="display:none"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"></path></svg>
        </button>
        <button class="icon-btn" id="open-settings" title="설정 파일 폴더 열기" aria-label="설정">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"></path></svg>
        </button>
      </div>
    `;
    this.inputEl_ = root.querySelector('#query')!;
    this.contentBtn = root.querySelector('#mode-content')!;
    this.filenameBtn = root.querySelector('#mode-filename')!;
    this.themeToggleBtn = root.querySelector('#theme-toggle')!;
    this.settingsBtn = root.querySelector('#open-settings')!;
    this.sunIcon = root.querySelector('#icon-sun')!;
    this.moonIcon = root.querySelector('#icon-moon')!;

    this.inputEl_.addEventListener('input', () => {
      this.dispatchEvent(new CustomEvent<string>('kd-query-input', { detail: this.inputEl_.value }));
    });
    this.contentBtn.addEventListener('click', () => this.setMode('content', true));
    this.filenameBtn.addEventListener('click', () => this.setMode('filename', true));
    this.themeToggleBtn.addEventListener('click', () => this.dispatchEvent(new CustomEvent('kd-theme-toggle')));
    this.settingsBtn.addEventListener('click', () => this.dispatchEvent(new CustomEvent('kd-open-settings')));
  }

  /** Shows the icon for the theme that clicking the button would switch *to*
   * (sun while dark is active, moon while light is active) - matches the
   * approved prototype. `main.ts` calls this after every theme change. */
  setThemeIcon(current: Theme): void {
    this.sunIcon.style.display = current === 'dark' ? '' : 'none';
    this.moonIcon.style.display = current === 'dark' ? 'none' : '';
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
