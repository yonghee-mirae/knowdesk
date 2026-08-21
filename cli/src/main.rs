//! 헤드리스 검증 하니스 (`docs/03_Architecture.md`). UI 없이 index/search/stats/bench를 구동한다.

use clap::{Parser, Subcommand, ValueEnum};
use knowdesk_core::config::Config;
use knowdesk_core::db::documents::DocumentRepository;
use knowdesk_core::db::Db;
use knowdesk_core::extract::ooxml::{DocxExtractor, PptxExtractor};
use knowdesk_core::extract::pdf::PdfExtractor;
use knowdesk_core::extract::txt::TxtExtractor;
use knowdesk_core::extract::xlsx::XlsxExtractor;
use knowdesk_core::extract::ContentExtractor;
use knowdesk_core::index::pipeline::IndexPipeline;
use knowdesk_core::index::queue;
use knowdesk_core::index::watcher::FileWatcher;
use knowdesk_core::nlp::bigram::BigramTokenizer;
use knowdesk_core::nlp::kiwi::KiwiTokenizer;
use knowdesk_core::nlp::Tokenizer;
use knowdesk_core::search::service::SqliteSearchService;
use knowdesk_core::search::{
    MatchKind, SearchMode as CoreSearchMode, SearchRequest, SearchService,
};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "cli", about = "KnowDesk 헤드리스 검증 하니스")]
struct Cli {
    /// 색인 DB 경로
    #[arg(long, global = true, default_value = "knowdesk.db")]
    db: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 폴더를 스캔해 색인한다.
    Index { path: PathBuf },
    /// 검색어로 검색한다.
    Search {
        query: String,
        #[arg(long, value_enum, default_value_t = ModeArg::Content)]
        mode: ModeArg,
        #[arg(long, default_value_t = 10)]
        limit: i64,
    },
    /// 색인 통계를 출력한다.
    Stats,
    /// 폴더를 계속 감시하며 변경을 즉시 색인한다 (Ctrl+C로 종료).
    Watch {
        path: PathBuf,
        /// 짧은 시간에 몰리는 이벤트를 하나로 합칠 시간 창 (밀리초).
        #[arg(long, default_value_t = 3000)]
        debounce_ms: u64,
    },
    /// 색인 처리량·검색 P95·DB 크기를 측정한다.
    Bench {
        path: PathBuf,
        /// 검색 벤치마크에 쓸 검색어 목록 파일 (한 줄에 하나). 생략하면 내부 기본
        /// 검색어 세트를 쓴다.
        #[arg(long)]
        queries: Option<PathBuf>,
        /// 검색어별 반복 횟수 (P95 안정화용).
        #[arg(long, default_value_t = 20)]
        repeat: usize,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ModeArg {
    Filename,
    Content,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config = Config {
        db_path: cli.db.clone(),
        ..Config::default()
    };
    let db = Db::open(&config.db_path)?;

    match cli.command {
        Command::Index { path } => run_index(&db, &config, &path)?,
        Command::Search { query, mode, limit } => run_search(&db, &query, mode, limit)?,
        Command::Stats => run_stats(&db)?,
        Command::Watch { path, debounce_ms } => run_watch(&db, &config, &path, debounce_ms)?,
        Command::Bench {
            path,
            queries,
            repeat,
        } => run_bench(&db, &config, &path, queries.as_deref(), repeat)?,
    }

    Ok(())
}

/// `KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`로 Kiwi를 초기화한다. bigram은
/// 항상 쓰는 기본 토크나이저라 실패해도 폴백할 필요가 없고, Kiwi는 되면 붙는 보조
/// 토크나이저라 안 되면 그냥 `None` — index/search 양쪽에서 공유한다.
fn load_kiwi() -> Option<KiwiTokenizer> {
    match KiwiTokenizer::from_env() {
        Some(Ok(kiwi)) => {
            tracing::info!("Kiwi 형태소 분석기 사용");
            Some(kiwi)
        }
        Some(Err(e)) => {
            tracing::warn!(error = %e, "Kiwi 초기화 실패, bigram만 사용");
            None
        }
        None => {
            tracing::info!("Kiwi 미설정, bigram만 사용");
            None
        }
    }
}

fn default_extractors() -> Vec<Box<dyn ContentExtractor>> {
    vec![
        Box::new(TxtExtractor),
        Box::new(XlsxExtractor),
        Box::new(DocxExtractor),
        Box::new(PptxExtractor),
        Box::new(PdfExtractor),
    ]
}

fn run_index(db: &Db, config: &Config, path: &Path) -> anyhow::Result<()> {
    let extractors = default_extractors();
    let bigram = BigramTokenizer;
    let kiwi = load_kiwi();
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: kiwi.as_ref().map(|k| k as &dyn Tokenizer),
    };
    let outcome = pipeline.index_directory(path)?;
    println!(
        "색인 완료 — 전체 {}건 중 본문 색인 {}건, 메타 색인 {}건, 스킵 {}건",
        outcome.full + outcome.meta + outcome.skip,
        outcome.full,
        outcome.meta,
        outcome.skip
    );
    Ok(())
}

fn run_watch(db: &Db, config: &Config, path: &Path, debounce_ms: u64) -> anyhow::Result<()> {
    // 감시 시작 전에 먼저 전체 스캔 — 감시 중에 놓친(꺼져 있던 동안의) 변경을
    // 반영하고, 이후엔 변경분만 반영한다.
    run_index(db, config, path)?;

    let extractors = default_extractors();
    let bigram = BigramTokenizer;
    let kiwi = load_kiwi();
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: kiwi.as_ref().map(|k| k as &dyn Tokenizer),
    };

    let watcher = FileWatcher::new(path, std::time::Duration::from_millis(debounce_ms))?;
    println!("변경 감시 중: {} (Ctrl+C로 종료)", path.display());
    while let Some(events) = watcher.recv() {
        for (path, result) in queue::drain(&pipeline, events) {
            match result {
                Ok(outcome) => println!("{}: {outcome:?}", path.display()),
                Err(e) => eprintln!("{}: 오류 {e}", path.display()),
            }
        }
    }
    Ok(())
}

fn run_search(db: &Db, query: &str, mode: ModeArg, limit: i64) -> anyhow::Result<()> {
    let kiwi = load_kiwi();
    let service = SqliteSearchService {
        conn: &db.conn,
        kiwi: kiwi.as_ref().map(|k| k as &dyn Tokenizer),
    };
    let request = SearchRequest {
        query: query.to_string(),
        mode: match mode {
            ModeArg::Filename => CoreSearchMode::Filename,
            ModeArg::Content => CoreSearchMode::Content,
        },
        limit,
    };
    let result = service.search(&request)?;

    if result.hits.is_empty() {
        println!("결과 없음");
        return Ok(());
    }

    for hit in result.hits {
        let tag = match hit.match_kind {
            MatchKind::Exact => "정확 일치",
            MatchKind::Morphological => "형태소 분석",
        };
        println!("{} [{tag}]", hit.path);
        if let Some(snippet) = hit.snippet {
            println!("  {snippet}");
        }
    }
    Ok(())
}

fn run_stats(db: &Db) -> anyhow::Result<()> {
    let tiers = DocumentRepository::count_by_tier(&db.conn)?;
    if tiers.is_empty() {
        println!("색인된 문서가 없습니다.");
        return Ok(());
    }
    for (tier, count) in tiers {
        println!("{tier}: {count}건");
    }
    for (reason, count) in DocumentRepository::count_by_demotion_reason(&db.conn)? {
        println!("  강등 사유 {reason}: {count}건");
    }
    Ok(())
}

/// `--queries` 파일이 없을 때 쓰는 기본 검색어 세트. `core/examples/gen_bench_corpus.rs`가
/// 생성하는 코퍼스의 문장 풀과 어휘를 맞춰뒀다 — 검색 종류(키워드/구문/AND/OR/NOT/접두)를
/// 한 번씩은 재도록 구성한다.
const DEFAULT_QUERIES: &[&str] = &[
    "채권",
    "\"이사회 결의\"",
    "채권 AND 발행",
    "채권 OR 예산",
    "채권 NOT 국채",
    "발행*",
];

/// PRD 성공 기준(`01_KnowDesk_PRD.md` 4장) 중 헤드리스로 실측 가능한 항목만 잰다.
/// "검색창 호출 P95 300ms"·유휴 CPU/메모리는 트레이·전역 단축키·상주 프로세스가
/// 있어야 실측 가능해서 Phase C/D 몫이다.
///
/// 색인은 매번 이 `path`를 새로 스캔하므로, 처리량 숫자가 의미 있으려면 `--db`가
/// 비어있는 상태로 실행해야 한다 (이미 색인된 db에 대고 돌리면 대부분 SKIP만 세게 된다).
fn run_bench(
    db: &Db,
    config: &Config,
    path: &Path,
    queries_file: Option<&Path>,
    repeat: usize,
) -> anyhow::Result<()> {
    let extractors = default_extractors();
    let bigram = BigramTokenizer;
    let kiwi = load_kiwi();
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: kiwi.as_ref().map(|k| k as &dyn Tokenizer),
    };

    let corpus_bytes = total_size(path)?;

    let start = std::time::Instant::now();
    let outcome = pipeline.index_directory(path)?;
    let elapsed = start.elapsed().as_secs_f64();
    let indexed = outcome.full + outcome.meta;
    println!(
        "색인: 전체 {}건 (본문 {} / 메타 {} / 스킵 {}), {elapsed:.2}초 ({:.1}건/초)",
        indexed + outcome.skip,
        outcome.full,
        outcome.meta,
        outcome.skip,
        indexed as f64 / elapsed.max(f64::EPSILON),
    );

    let queries = load_queries(queries_file)?;
    let service = SqliteSearchService {
        conn: &db.conn,
        kiwi: kiwi.as_ref().map(|k| k as &dyn Tokenizer),
    };
    let mut latencies_ms = Vec::with_capacity(queries.len() * repeat);
    for query in &queries {
        let request = SearchRequest {
            query: query.clone(),
            mode: CoreSearchMode::Content,
            limit: 10,
        };
        for _ in 0..repeat {
            let t0 = std::time::Instant::now();
            service.search(&request)?;
            latencies_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
    }
    latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = percentile(&latencies_ms, 50.0);
    let p95 = percentile(&latencies_ms, 95.0);
    let verdict = if p95 <= 1000.0 { "PASS" } else { "FAIL" };
    println!(
        "검색: {}개 쿼리 × {repeat}회\n  P50 {p50:.1}ms / P95 {p95:.1}ms (기준 1000ms 이내 — {verdict})",
        queries.len(),
    );

    let db_size = std::fs::metadata(&config.db_path)?.len();
    println!(
        "DB 크기: {} (원본 {} 대비 {:.2}배)",
        format_bytes(db_size),
        format_bytes(corpus_bytes),
        db_size as f64 / corpus_bytes.max(1) as f64,
    );

    Ok(())
}

/// 정렬된 값 목록에서 p백분위(0~100)를 구한다.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

/// `path` 파일이 있으면 한 줄에 하나씩 읽고, 없으면 기본 검색어 세트를 쓴다.
fn load_queries(path: Option<&Path>) -> anyhow::Result<Vec<String>> {
    match path {
        Some(p) => Ok(std::fs::read_to_string(p)?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()),
        None => Ok(DEFAULT_QUERIES.iter().map(|s| s.to_string()).collect()),
    }
}

/// `root` 아래 파일 전체의 크기 합 — 색인 대상 필터(임시파일/확장자 제외 등)는
/// 적용하지 않고 폴더 안 모든 파일을 그대로 더한다("원본 용량"의 직관적인 의미).
fn total_size(root: &Path) -> anyhow::Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                stack.push(entry.path());
            } else {
                total += entry.metadata()?.len();
            }
        }
    }
    Ok(total)
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1}{}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_of_empty_is_zero() {
        assert_eq!(percentile(&[], 95.0), 0.0);
    }

    #[test]
    fn percentile_picks_expected_rank() {
        // 값 1..=100, 인덱스 0..=99. rank = round(p/100 * (len-1)).
        let sorted: Vec<f64> = (1..=100).map(|n| n as f64).collect();
        assert_eq!(percentile(&sorted, 50.0), 51.0); // round(0.50*99)=50 → sorted[50]
        assert_eq!(percentile(&sorted, 95.0), 95.0); // round(0.95*99)=94 → sorted[94]
        assert_eq!(percentile(&sorted, 100.0), 100.0); // round(1.00*99)=99 → sorted[99]
    }

    #[test]
    fn format_bytes_picks_unit() {
        assert_eq!(format_bytes(512), "512.0B");
        assert_eq!(format_bytes(2048), "2.0KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0MB");
    }
}
