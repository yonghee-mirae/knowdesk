// Search-syntax help content shown in place of the result list while the
// query is empty (`docs/12_UI_Spec.md` C1, `docs/05_Search_Language_v1.md`).
// Ported verbatim from the approved prototype - every example here was
// checked against the real backend during that round of work.

import type { SearchMode } from '../types';

export interface HelpPart {
  t: string;
  /** 'k' = syntax itself (operator/filter key - type it exactly as shown).
   * 'v' = a placeholder for the user's own search term/value. Omitted for
   * plain connective text (spaces, parens shown only for display). */
  r?: 'k' | 'v';
}

export interface HelpItem {
  group: string;
  desc: string;
  parts: HelpPart[];
}

export const MODE_DESC: Record<SearchMode, string> = {
  content: '문서 본문에서 찾습니다. 형태소 검색이 가능합니다.',
  filename: '파일명에서 찾습니다. 입력한 단어가 모두 포함된 파일명을 찾습니다.',
};

const FILTER_HELP: HelpItem[] = [
  { group: '필터', desc: '확장자로 좁힙니다.', parts: [{ t: 'x:', r: 'k' }, { t: 'pdf', r: 'v' }] },
  {
    group: '필터',
    desc: '경로에 이 문자열이 포함된 문서만 찾습니다.',
    parts: [{ t: 'p:', r: 'k' }, { t: '리서치', r: 'v' }],
  },
  {
    group: '필터',
    desc: '이 날짜 이후에 수정된 문서만 찾습니다.',
    parts: [{ t: 'm>', r: 'k' }, { t: '2026-01-01', r: 'v' }],
  },
  {
    group: '필터',
    desc: '이 날짜 이전에 수정된 문서만 찾습니다.',
    parts: [{ t: 'm<', r: 'k' }, { t: '2026-08-01', r: 'v' }],
  },
  {
    group: '필터',
    desc: '정확히 이 날짜에 수정된 문서만 찾습니다.',
    parts: [{ t: 'm=', r: 'k' }, { t: '2026-08-10', r: 'v' }],
  },
];

// Content mode: matched against the extracted body (bigram/Kiwi), so a plain
// keyword behaves like a substring match within a longer word (e.g. "발행"
// inside "재발행").
const CONTENT_SEARCH_ONLY: HelpItem[] = [
  {
    group: '검색어',
    desc: '두 단어가 모두 포함된 문서를 찾습니다. 단어 사이 공백은 AND와 같은 뜻이라 생략해도 됩니다.',
    parts: [{ t: '채권', r: 'v' }, { t: ' (' }, { t: 'AND', r: 'k' }, { t: ') ' }, { t: '발행', r: 'v' }],
  },
  {
    group: '검색어',
    desc: '둘 중 하나라도 포함하면 찾습니다.',
    parts: [{ t: '채권', r: 'v' }, { t: ' ' }, { t: 'OR', r: 'k' }, { t: ' ' }, { t: '회사채', r: 'v' }],
  },
  {
    group: '검색어',
    desc: '앞 단어는 포함하고 뒷 단어는 제외합니다.',
    parts: [{ t: '채권', r: 'v' }, { t: ' ' }, { t: 'NOT', r: 'k' }, { t: ' ' }, { t: '국채', r: 'v' }],
  },
  {
    group: '검색어',
    desc: '괄호로 우선순위를 정합니다. 괄호 없으면 AND가 OR보다 먼저 묶입니다.',
    parts: [
      { t: '(', r: 'k' },
      { t: '채권', r: 'v' },
      { t: ' ' },
      { t: 'OR', r: 'k' },
      { t: ' ' },
      { t: '회사채', r: 'v' },
      { t: ')', r: 'k' },
      { t: ' ' },
      { t: 'AND', r: 'k' },
      { t: ' ' },
      { t: '발행', r: 'v' },
    ],
  },
  {
    group: '검색어',
    desc: '띄어쓰기까지 정확히 일치하는 구문만 찾습니다.',
    parts: [{ t: '"', r: 'k' }, { t: '채권 발행', r: 'v' }, { t: '"', r: 'k' }],
  },
  {
    group: '검색어',
    desc: 'AND라는 단어 자체를 찾습니다 (연산자가 아니라 리터럴로).',
    parts: [{ t: '"', r: 'k' }, { t: 'AND', r: 'v' }, { t: '"', r: 'k' }],
  },
  {
    group: '검색어',
    desc: '"발행"으로 시작하는 모든 단어를 찾습니다.',
    parts: [{ t: '발행', r: 'v' }, { t: '*', r: 'k' }],
  },
];

// Filename mode is a plain SQL substring ("contains") match on `paths.filename`,
// not FTS5 (`docs/05_Search_Language_v1.md` Filename Mode, `core/src/search/parser.rs`'s
// `parse_filename`) - AND/OR/NOT, quotes, and the trailing-`*` wildcard have no
// special meaning here, so there's no search-term syntax left worth a help row
// beyond "type part of the name." Only the shared filters below still apply.
export const CONTENT_SEARCH_HELP: HelpItem[] = [...CONTENT_SEARCH_ONLY, ...FILTER_HELP];
export const FILENAME_SEARCH_HELP: HelpItem[] = [...FILTER_HELP];
