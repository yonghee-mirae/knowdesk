// Settings window (`docs/12_UI_Spec.md` C5, TASK-704). Scoped to just the
// "색인 대상 폴더" list for now - the mockup's other fields (제외 패턴,
// 전역 단축키 변경, 검색 결과 개수, 자동 실행, DB 위치, 색인 초기화) are
// deliberately out of scope until there's a concrete need for them.

import { initTheme } from './core/theme';
import * as backend from './platform/backend';

initTheme();

const maybeListEl = document.querySelector<HTMLUListElement>('#folder-list');
const maybeAddBtn = document.querySelector<HTMLButtonElement>('#add-folder');
if (!maybeListEl || !maybeAddBtn) {
  throw new Error('KnowDesk settings: required elements missing from settings.html');
}
const listEl = maybeListEl;
const addBtn = maybeAddBtn;

function renderFolders(folders: string[]): void {
  listEl.innerHTML = '';
  if (folders.length === 0) {
    const empty = document.createElement('li');
    empty.className = 'empty';
    empty.textContent = '등록된 폴더가 없습니다.';
    listEl.appendChild(empty);
    return;
  }
  for (const folder of folders) {
    const item = document.createElement('li');
    const path = document.createElement('span');
    path.className = 'path';
    path.textContent = folder;
    path.title = folder;
    const removeBtn = document.createElement('button');
    removeBtn.textContent = '제거';
    removeBtn.addEventListener('click', () => void removeFolder(folder));
    item.append(path, removeBtn);
    listEl.appendChild(item);
  }
}

async function removeFolder(path: string): Promise<void> {
  renderFolders(await backend.removeWatchedFolder(path));
}

addBtn.addEventListener('click', () => {
  void (async () => {
    // Disabled for the round trip so a slow dialog/double-click can't fire
    // `add_watched_folder` twice for the same pick.
    addBtn.disabled = true;
    try {
      const picked = await backend.openFolderPicker();
      if (picked) {
        renderFolders(await backend.addWatchedFolder(picked));
      }
    } finally {
      addBtn.disabled = false;
    }
  })();
});

void backend.getWatchedFolders().then(renderFolders);
