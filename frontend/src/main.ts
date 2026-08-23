import './components/kd-search-bar';
import './components/kd-result-list';
import './components/kd-syntax-help';
import './components/kd-preview';
import { getCurrentWindow } from '@tauri-apps/api/window';
import * as backend from './platform/backend';
import { loadAndApplyTheme } from './core/theme';
import { MOD_KEY } from './core/platform';
import type { KdSearchBar } from './components/kd-search-bar';
import type { KdResultList } from './components/kd-result-list';
import type { KdSyntaxHelp } from './components/kd-syntax-help';
import type { KdPreview } from './components/kd-preview';
import type { SearchHit, SearchMode } from './types';

void loadAndApplyTheme();
void loadResultLimit();
void loadSearchDebounceMs();
// Re-checks on every focus (shown via the tray/hotkey) - see
// `loadAndApplyTheme`'s doc comment for why that's needed instead of a
// push-based update. Same reasoning applies to `resultLimit`/`searchDebounceMs`.
window.addEventListener('focus', () => {
  void loadAndApplyTheme();
  void loadResultLimit();
  void loadSearchDebounceMs();
});

// The footer hint bar's "폴더 열기"/"경로 복사" shortcuts are hardcoded as
// "Ctrl" in index.html - swap to "⌘" on macOS to match the actual modifier
// used there (`docs/12_UI_Spec.md` C1 targets Windows/Ctrl as the default).
document.querySelectorAll('.kd-footer kbd').forEach((el) => {
  if (el.textContent === 'Ctrl') el.textContent = MOD_KEY;
});

// Fallbacks until each loader's first read resolves - match the backend's own
// defaults (`core::config::DEFAULT_RESULT_LIMIT`/`DEFAULT_SEARCH_DEBOUNCE_MS`).
// `0` means unlimited, same convention as the backend field.
let resultLimit = 0;
let debounceMs = 150;

/** Reads `result_limit` from `settings.json` (via the backend). Best-effort -
 * an unreadable/corrupt settings.json shouldn't block search, just keeps
 * whatever value was already in effect. */
async function loadResultLimit(): Promise<void> {
  try {
    resultLimit = await backend.getResultLimit();
  } catch {
    // Keep the previous value.
  }
}

/** Reads `search_debounce_ms` from `settings.json` (via the backend).
 * Best-effort, same reasoning as `loadResultLimit`. */
async function loadSearchDebounceMs(): Promise<void> {
  try {
    debounceMs = await backend.getSearchDebounceMs();
  } catch {
    // Keep the previous value.
  }
}

const maybeBody = document.querySelector<HTMLDivElement>('#body');
const maybeSearchBar = document.querySelector<KdSearchBar>('kd-search-bar');
const maybeResultList = document.querySelector<KdResultList>('kd-result-list');
const maybeSyntaxHelp = document.querySelector<KdSyntaxHelp>('kd-syntax-help');
const maybePreview = document.querySelector<KdPreview>('kd-preview');
if (!maybeBody || !maybeSearchBar || !maybeResultList || !maybeSyntaxHelp || !maybePreview) {
  throw new Error('KnowDesk: required elements missing from index.html');
}
const body = maybeBody;
const searchBar = maybeSearchBar;
const resultList = maybeResultList;
const syntaxHelp = maybeSyntaxHelp;
const preview = maybePreview;

const state = {
  mode: 'content' as SearchMode,
  hits: [] as SearchHit[],
  selected: 0,
};

/** The syntax help has nothing to preview, so an empty query collapses to a
 * single full-width pane instead of a result list next to an empty
 * placeholder (`docs/12_UI_Spec.md` C1, ported from the approved prototype). */
function showSyntaxHelp(): void {
  body.classList.add('single-pane');
  resultList.hidden = true;
  preview.hidden = true;
  syntaxHelp.hidden = false;
  syntaxHelp.render(state.mode);
}

function showResults(): void {
  body.classList.remove('single-pane');
  syntaxHelp.hidden = true;
  resultList.hidden = false;
  preview.hidden = false;
}

/** Shows `hit` in the preview pane, and - if it has no snippet (a
 * filter-only query, or filename mode never has one) and its body was
 * actually extracted (FULL tier) - fetches the document's opening text and
 * fills it in once it arrives, instead of leaving the pane empty. The result
 * list doesn't need this: there's no match there either way to show
 * (`docs/12_UI_Spec.md` C2). */
function showPreview(hit: SearchHit): void {
  preview.showHit(hit);
  if (hit.snippet === null && hit.indexTier === 'FULL') {
    void backend.previewBody(hit.path).then((text) => preview.showBodyPreview(hit.path, text));
  }
}

// Guards against out-of-order replies when a fast typist outruns the debounce
// (e.g. two keystrokes each trigger a search, and the first one's IPC round
// trip resolves after the second one's) - only the most recently issued
// search's result is ever applied.
let searchSeq = 0;

async function runSearch(): Promise<void> {
  const seq = ++searchSeq;
  const query = searchBar.value.trim();
  if (!query) {
    state.hits = [];
    showSyntaxHelp();
    return;
  }

  const hits = await backend.search(query, state.mode, resultLimit);
  if (seq !== searchSeq) return; // A newer search has since started - discard this one.

  state.hits = hits;
  state.selected = 0;
  showResults();
  if (hits.length === 0) {
    resultList.render([], state.mode, { hint: '검색 결과가 없습니다', sub: '다른 검색어를 시도해보세요' });
    preview.clear();
    return;
  }
  resultList.render(hits, state.mode, null);
  showPreview(hits[0] as SearchHit);
}

let debounceHandle: ReturnType<typeof setTimeout> | null = null;
// The currently-running `runSearch()` call, if any - tracked separately from
// `debounceHandle` because the real backend search is an async IPC round
// trip (unlike the prototype's synchronous mock), so a search can still be
// in flight even after its debounce timer has already fired and cleared
// `debounceHandle`.
let inFlightSearch: Promise<void> | null = null;

function launchSearch(): void {
  inFlightSearch = runSearch().finally(() => {
    inFlightSearch = null;
  });
}

function scheduleSearch(): void {
  if (debounceHandle !== null) clearTimeout(debounceHandle);
  debounceHandle = setTimeout(() => {
    debounceHandle = null;
    launchSearch();
  }, debounceMs);
}

// Arrow-key/Enter/Ctrl+C handling reads `state.hits`/`state.selected` right
// after this returns - awaiting both a still-pending debounce AND any
// already-in-flight search is required so a slower keystroke-triggered
// search can't resolve *after* the key's own selection update and silently
// revert it (`resultList.render()`/`runSearch()` reset the selection to 0).
async function flushPendingSearch(): Promise<void> {
  if (debounceHandle !== null) {
    clearTimeout(debounceHandle);
    debounceHandle = null;
    launchSearch();
  }
  if (inFlightSearch) {
    await inFlightSearch;
  }
}

/** Clears the query and returns to the empty (syntax-help) state - used only
 * by Esc. Unlike the global hotkey's toggle (which just hides the window via
 * Rust and leaves the webview's state untouched, so reopening finds the
 * previous query still there), Esc means "dismiss this search," so it resets
 * before hiding. Bumping `searchSeq` invalidates any debounced/in-flight
 * search for the query being cleared, so a slow reply can't land afterwards
 * and repopulate results the reset just cleared. */
function resetSearch(): void {
  if (debounceHandle !== null) {
    clearTimeout(debounceHandle);
    debounceHandle = null;
  }
  searchSeq++;
  searchBar.clear();
  state.hits = [];
  state.selected = 0;
  showSyntaxHelp();
}

function setMode(mode: SearchMode): void {
  if (state.mode === mode) return;
  state.mode = mode;
  searchBar.setMode(mode);
  launchSearch();
}

function moveSelection(delta: number): void {
  if (state.hits.length === 0) return;
  state.selected = (state.selected + delta + state.hits.length) % state.hits.length;
  resultList.setSelected(state.selected);
  showPreview(state.hits[state.selected] as SearchHit);
}

function selectIndex(index: number): void {
  if (index < 0 || index >= state.hits.length) return;
  state.selected = index;
  resultList.setSelected(index);
  showPreview(state.hits[index] as SearchHit);
}

searchBar.addEventListener('kd-query-input', () => scheduleSearch());
searchBar.addEventListener('kd-mode-change', (e) => setMode((e as CustomEvent<SearchMode>).detail));
resultList.addEventListener('kd-row-click', (e) => selectIndex((e as CustomEvent<number>).detail));

window.addEventListener('keydown', (e) => {
  const withMod = e.ctrlKey || e.metaKey;

  if (withMod && e.key === '1') {
    e.preventDefault();
    setMode('content');
    return;
  }
  if (withMod && e.key === '2') {
    e.preventDefault();
    setMode('filename');
    return;
  }

  // preventDefault() must run synchronously, before the `await` below
  // suspends this handler and hands control back to the event dispatcher -
  // it has no effect once called after that point.
  if (e.key === 'ArrowDown') {
    e.preventDefault();
    void flushPendingSearch().then(() => moveSelection(1));
    return;
  }
  if (e.key === 'ArrowUp') {
    e.preventDefault();
    void flushPendingSearch().then(() => moveSelection(-1));
    return;
  }

  if (e.key === 'Enter') {
    e.preventDefault();
    void flushPendingSearch().then(() => {
      const hit = state.hits[state.selected];
      if (!hit) return;
      void (withMod ? backend.openParentFolder(hit.path) : backend.openPath(hit.path));
    });
    return;
  }

  if (withMod && e.key.toLowerCase() === 'c') {
    // The search input always holds real DOM focus (see the mousedown guard
    // below), so "포커스가 결과 리스트에 있을 때만" (`docs/12_UI_Spec.md` C1) is
    // read as "the input has no text selection to copy instead" - otherwise
    // Ctrl+C still copies the selected query text, as expected.
    if (searchBar.inputEl.selectionStart !== searchBar.inputEl.selectionEnd) return;
    if (!state.hits[state.selected]) return;
    e.preventDefault();
    void flushPendingSearch().then(() => {
      const hit = state.hits[state.selected];
      if (hit) void backend.copyPath(hit.path);
    });
    return;
  }

  if (e.key === 'Escape') {
    e.preventDefault();
    resetSearch();
    void getCurrentWindow().hide();
  }
});

// Spotlight-style: the caret always stays in the search input - clicking a
// result row or a mode-toggle button must not steal it. Done on `mousedown`
// with `preventDefault()` rather than reacting to `focusin` and calling
// `.focus()` back: re-focusing an already-focused input resets its text
// caret, which fights with arrow-key navigation re-rendering the list on
// every press. `preventDefault()` here stops focus from moving in the first
// place - the `click`/keydown handlers above still fire normally.
document.addEventListener('mousedown', (e) => {
  if (e.composedPath()[0] === searchBar.inputEl) return;
  e.preventDefault();
});

showSyntaxHelp();
searchBar.focusInput();
