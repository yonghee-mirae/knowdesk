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
    /// 색인/검색 벤치마크. Phase B(B5)에서 구현 예정.
    Bench,
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
        Command::Bench => println!("bench는 Phase B(B5)에서 구현 예정입니다."),
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

fn run_index(db: &Db, config: &Config, path: &Path) -> anyhow::Result<()> {
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![
        Box::new(TxtExtractor),
        Box::new(XlsxExtractor),
        Box::new(DocxExtractor),
        Box::new(PptxExtractor),
        Box::new(PdfExtractor),
    ];
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
