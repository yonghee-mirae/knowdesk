//! 파일 시스템 감시 + 디바운스 직접 구현 (Phase B4, `docs/06_Development_Roadmap.md`).
//!
//! `notify-debouncer-mini`(공식 동반 크레이트)로 먼저 구현했다가, **무한 색인
//! 루프**를 실제로 재현해서 되돌렸다: Linux inotify 백엔드는 기본적으로 `OPEN`/
//! `ATTRIB`(접근/메타데이터 변경) 이벤트까지 감시하는데, 우리 색인 파이프라인이
//! 파일을 읽는 것(해시 계산, 텍스트 추출) 자체가 `OPEN` 이벤트를 만든다.
//! `notify-debouncer-mini`는 이벤트 종류를 구분해 넘겨주지 않아서 "색인하려고
//! 읽음 → 그 읽기가 이벤트를 만듦 → 다시 색인" 무한 루프가 됐다(실제로 확인함,
//! `notify` 소스의 `WatchMask::ATTRIB | CREATE | OPEN | DELETE | CLOSE_WRITE |
//! MODIFY | MOVED_FROM | MOVED_TO` 참조).
//!
//! 그래서 원시 `notify::Event`를 직접 받아 `EventKind`로 필터링한다 — 생성/삭제/
//! 실제 내용 변경(Data)/이름 변경(Name)만 반응하고, 접근(Access)·메타데이터
//! 변경(Metadata, atime 등)은 우리 자신의 읽기가 만들어내는 노이즈라 무시한다.
//! 디바운스는 "첫 이벤트 이후, `debounce` 동안 같은 경로에 새 이벤트가 없으면
//! 확정"하는 단순한 방식으로 직접 구현한다.

use notify::event::ModifyKind;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

pub struct FileWatcher {
    // Drop되면 감시가 멈추므로 계속 들고 있어야 한다.
    _watcher: RecommendedWatcher,
    raw_events: Receiver<Event>,
    debounce: Duration,
}

impl FileWatcher {
    /// `root` 아래를 재귀적으로 감시한다. `debounce`는 마지막 이벤트 이후 이만큼
    /// 조용해야 변경을 확정하는 시간 창이다.
    pub fn new(root: &Path, debounce: Duration) -> notify::Result<Self> {
        let (tx, raw_events) = channel();
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                if let Ok(event) = res {
                    if is_relevant(&event.kind) {
                        let _ = tx.send(event);
                    }
                }
            },
            Config::default(),
        )?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self {
            _watcher: watcher,
            raw_events,
            debounce,
        })
    }

    /// 디바운스된 변경 경로 목록을 하나 기다려 받는다 (블로킹). 감시가 끝나면
    /// (채널 닫힘) `None`.
    pub fn recv(&self) -> Option<Vec<PathBuf>> {
        let first = self.raw_events.recv().ok()?;
        Some(self.settle(first))
    }

    /// `recv`와 같지만 첫 이벤트를 `wait`까지만 기다린다 — 그 안에 아무 일도
    /// 없으면 `None`. "더 이상 이벤트가 없다"를 확인해야 할 때 쓴다(예: 무한
    /// 재색인 루프 회귀 테스트).
    pub fn recv_timeout(&self, wait: Duration) -> Option<Vec<PathBuf>> {
        let first = self.raw_events.recv_timeout(wait).ok()?;
        Some(self.settle(first))
    }

    /// 이미 받은 첫 이벤트 이후, 조용해질 때까지(`debounce`) 같은 경로의 새
    /// 이벤트를 계속 모은다.
    fn settle(&self, first: Event) -> Vec<PathBuf> {
        let mut pending: HashSet<PathBuf> = first.paths.into_iter().collect();
        while let Ok(event) = self.raw_events.recv_timeout(self.debounce) {
            pending.extend(event.paths);
        }
        pending.into_iter().collect()
    }
}

/// 우리가 실제로 반응해야 할 이벤트 종류만 남긴다. `Access`(열기/닫기)와
/// `Modify(Metadata)`(atime 등)는 색인 파이프라인의 읽기 자체가 만들어내는
/// 노이즈이므로 제외한다.
fn is_relevant(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Name(_))
    )
}
