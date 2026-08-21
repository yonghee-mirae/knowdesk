//! Phase B4 완료 기준 통합 검증: 파일 생성/삭제를 실시간으로 반영하고, 색인
//! 파이프라인 자신의 파일 읽기가 다시 이벤트를 만들어 무한 재색인되지 않는다.

use knowdesk_core::config::Config;
use knowdesk_core::db::Db;
use knowdesk_core::extract::txt::TxtExtractor;
use knowdesk_core::extract::ContentExtractor;
use knowdesk_core::index::pipeline::IndexPipeline;
use knowdesk_core::index::queue::{self, WatchOutcome};
use knowdesk_core::index::watcher::FileWatcher;
use knowdesk_core::nlp::bigram::BigramTokenizer;
use knowdesk_core::search::service::SqliteSearchService;
use knowdesk_core::search::{SearchMode, SearchRequest, SearchService};
use std::time::Duration;

const DEBOUNCE: Duration = Duration::from_millis(200);
// 이벤트를 기다리는 최대 시간. 디바운스보다 넉넉해야 한다.
const WAIT: Duration = Duration::from_secs(5);

#[test]
fn same_file_via_different_path_strings_is_treated_as_one_document() {
    // 실제로 있었던 버그: `cli watch`의 최초 전체 스캔은 사용자가 준 경로를
    // 그대로 쓰지만(예: "./samples/x.txt"), 그 뒤 `notify`가 변경을 알릴 땐
    // 현재 작업 디렉터리를 붙인 절대 경로로 이벤트를 준다
    // ("/현재/디렉터리/./samples/x.txt"). canonicalize 없이는 이 둘이 다른
    // 파일로 취급돼 같은 파일이 문서 두 개로 나뉘어 색인되고, 내용을 수정해도
    // 예전 내용이 검색에 영구히 남았다. 여기서는 cwd를 바꾸지 않고도 같은
    // 문제를 재현하기 위해, 중간에 "./"가 낀 다른 문자열로 같은 파일을 가리킨다
    // (실제 버그의 경로 모양과 동일).
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("보고서.txt");
    std::fs::write(&file_path, "GDP 성장률은 3.2%였다.").unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let bigram = BigramTokenizer;
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: None,
    };

    pipeline.index_file(&file_path).unwrap();

    std::fs::write(&file_path, "GDP 성장률은 5.2%였다.").unwrap();
    let differently_written_path = dir.path().join(".").join(file_path.file_name().unwrap());
    // `PathBuf`의 `==`는 컴포넌트 단위로 비교해서 중간의 "."을 무시하므로 같다고
    // 나온다 — 문제는 `path.to_string_lossy()`로 DB 키를 만들 때 이 문자열
    // 표현이 다르다는 것이다(실제 버그의 원인).
    assert_ne!(
        file_path.to_string_lossy(),
        differently_written_path.to_string_lossy(),
        "테스트 전제가 깨짐: 두 경로의 문자열 표현이 원래 같으면 안 됨"
    );
    pipeline.index_file(&differently_written_path).unwrap();

    let path_count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM paths", [], |row| row.get(0))
        .unwrap();
    assert_eq!(path_count, 1, "같은 파일이 경로 두 개로 나뉘어 색인됨");

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let old_content = search
        .search(&SearchRequest {
            query: "3.2%".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();
    assert!(
        old_content.hits.is_empty(),
        "예전 내용이 여전히 검색됨: {:?}",
        old_content.hits
    );

    let new_content = search
        .search(&SearchRequest {
            query: "5.2%".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();
    assert_eq!(new_content.hits.len(), 1, "히트: {:?}", new_content.hits);
}

#[test]
fn watch_indexes_new_file_and_removes_deleted_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let bigram = BigramTokenizer;
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: None,
    };

    let watcher = FileWatcher::new(dir.path(), DEBOUNCE).unwrap();

    let file_path = dir.path().join("규정.txt");
    std::fs::write(&file_path, "채권 발행 절차").unwrap();

    let events = watcher.recv_timeout(WAIT).expect("생성 이벤트를 못 받음");
    let outcomes = queue::drain(&pipeline, events);
    assert_eq!(outcomes.len(), 1, "이벤트: {:?}", outcomes);
    assert!(matches!(outcomes[0].1, Ok(WatchOutcome::Indexed(_))));

    // 색인이 실제로 검색 가능한지 확인한다.
    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let result = search
        .search(&SearchRequest {
            query: "채권".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();
    assert_eq!(result.hits.len(), 1, "히트: {:?}", result.hits);

    // 색인 파이프라인이 방금 이 파일을 읽었다(해시 계산, 텍스트 추출) — 그
    // 읽기 자체가 새 이벤트를 만들어 무한 재색인되면 안 된다(실제로 있었던
    // 버그, `watcher.rs` 참조). 충분히 기다려도 더 이상 이벤트가 없어야 한다.
    assert!(
        watcher.recv_timeout(WAIT).is_none(),
        "색인 파이프라인의 읽기 자체가 이벤트를 만들어 재색인 루프가 도는 것으로 보임"
    );

    // 삭제하면 색인에서 지워져야 한다.
    std::fs::remove_file(&file_path).unwrap();
    let events = watcher.recv_timeout(WAIT).expect("삭제 이벤트를 못 받음");
    let outcomes = queue::drain(&pipeline, events);
    assert_eq!(outcomes.len(), 1, "이벤트: {:?}", outcomes);
    assert!(matches!(outcomes[0].1, Ok(WatchOutcome::Removed)));

    let result = search
        .search(&SearchRequest {
            query: "채권".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();
    assert!(
        result.hits.is_empty(),
        "삭제 후에도 검색됨: {:?}",
        result.hits
    );
}

#[test]
fn watch_ignores_short_lived_temp_file_from_office_style_save() {
    // Office가 저장할 때 임시파일(`~$파일명`)이 생성되고 곧 지워지는 것을
    // 흉내낸다. 디바운스 창 안에서 사라지면 색인 시도 자체가 없어야 한다.
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let bigram = BigramTokenizer;
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: None,
    };

    let watcher = FileWatcher::new(dir.path(), DEBOUNCE).unwrap();

    let temp_path = dir.path().join("~$규정.txt");
    let real_path = dir.path().join("규정.txt");
    std::fs::write(&temp_path, "임시").unwrap();
    std::fs::remove_file(&temp_path).unwrap();
    std::fs::write(&real_path, "본 문서는 채권 발행 절차를 규정한다.").unwrap();

    let events = watcher.recv_timeout(WAIT).expect("이벤트를 못 받음");
    let outcomes = queue::drain(&pipeline, events);

    let real_outcome = outcomes
        .iter()
        .find(|(path, _)| path == &real_path)
        .expect("실제 파일 이벤트가 없음");
    assert!(matches!(real_outcome.1, Ok(WatchOutcome::Indexed(_))));

    if let Some((_, temp_outcome)) = outcomes.iter().find(|(path, _)| path == &temp_path) {
        // 임시파일 이벤트가 같이 왔더라도, 색인 전에 이미 사라졌으니 무시돼야
        // 한다 — 확장자 자체는 지원 대상(.txt)이라 파일이 그대로 있었다면
        // SKIP이 아니라 색인됐을 것이다.
        assert!(
            matches!(temp_outcome, Ok(WatchOutcome::Ignored)),
            "임시파일 처리 결과: {:?}",
            temp_outcome
        );
    }
}
