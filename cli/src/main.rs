//! Headless verification harness (`docs/03_Architecture.md`). Runs
//! index/search/stats/bench without a UI.

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
#[command(name = "cli", about = "KnowDesk headless verification harness")]
struct Cli {
    /// Path to the index DB
    #[arg(long, global = true, default_value = "knowdesk.db")]
    db: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scans a folder and indexes it.
    Index { path: PathBuf },
    /// Searches using a query.
    Search {
        query: String,
        #[arg(long, value_enum, default_value_t = ModeArg::Content)]
        mode: ModeArg,
        #[arg(long, default_value_t = 10)]
        limit: i64,
    },
    /// Prints index statistics.
    Stats,
    /// Continuously watches a folder and indexes changes immediately (Ctrl+C
    /// to quit).
    Watch {
        path: PathBuf,
        /// Time window (in milliseconds) for coalescing events that arrive
        /// in a short burst.
        #[arg(long, default_value_t = 3000)]
        debounce_ms: u64,
    },
    /// Measures indexing throughput, search P95, and DB size.
    Bench {
        path: PathBuf,
        /// File of search queries to use for the search benchmark (one per
        /// line). If omitted, uses the built-in default query set.
        #[arg(long)]
        queries: Option<PathBuf>,
        /// Number of repetitions per query (to stabilize P95).
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

/// Initializes Kiwi from `KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`.
/// Bigram is always the default tokenizer in use, so there's no need to fall
/// back if it fails; Kiwi is an optional secondary tokenizer that's added
/// when available and just `None` when it's not — shared between both index
/// and search.
fn load_kiwi() -> Option<KiwiTokenizer> {
    let kiwi = match KiwiTokenizer::from_env() {
        Some(Ok(kiwi)) => {
            tracing::info!("Using Kiwi morphological analyzer");
            Some(kiwi)
        }
        Some(Err(e)) => {
            tracing::warn!(error = %e, "Kiwi initialization failed, using bigram only");
            None
        }
        None => {
            tracing::info!("Kiwi not configured, using bigram only");
            None
        }
    };
    // `tracing` output is only visible when RUST_LOG is set, so unlike the log lines
    // above, print a notice unconditionally — the user should always know when
    // morphological search isn't active, not just when they happen to have logging on.
    if kiwi.is_none() {
        eprintln!(
            "Notice: Kiwi morphological analyzer is not available — using bigram tokenization only."
        );
    }
    kiwi
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
        "Indexing complete — {} total, {} full-text indexed, {} metadata indexed, {} skipped",
        outcome.full + outcome.meta + outcome.skip,
        outcome.full,
        outcome.meta,
        outcome.skip
    );
    Ok(())
}

fn run_watch(db: &Db, config: &Config, path: &Path, debounce_ms: u64) -> anyhow::Result<()> {
    // Do a full scan before starting the watch — this picks up changes
    // missed while watching was off, and afterwards only diffs are applied.
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

    let watcher = FileWatcher::new(&[path], std::time::Duration::from_millis(debounce_ms))?;
    println!("Watching for changes: {} (Ctrl+C to quit)", path.display());
    while let Some(events) = watcher.recv() {
        for (path, result) in queue::drain(&pipeline, events) {
            match result {
                Ok(outcome) => println!("{}: {outcome:?}", path.display()),
                Err(e) => eprintln!("{}: error {e}", path.display()),
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
        println!("No results");
        return Ok(());
    }

    for hit in result.hits {
        let tag = match hit.match_kind {
            MatchKind::Exact => "exact match",
            MatchKind::Morphological => "morphological match",
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
        println!("No documents indexed.");
        return Ok(());
    }
    for (tier, count) in tiers {
        println!("{tier}: {count} docs");
    }
    for (reason, count) in DocumentRepository::count_by_demotion_reason(&db.conn)? {
        println!("  demotion reason {reason}: {count} docs");
    }
    Ok(())
}

/// Default query set used when the `--queries` file is absent. Matched to
/// the sentence pool and vocabulary of the corpus generated by
/// `core/examples/gen_bench_corpus.rs` — arranged to exercise each search
/// kind (keyword/phrase/AND/OR/NOT/prefix) at least once.
const DEFAULT_QUERIES: &[&str] = &[
    "채권",
    "\"이사회 결의\"",
    "채권 AND 발행",
    "채권 OR 예산",
    "채권 NOT 국채",
    "발행*",
];

/// Measures only the PRD success criteria (`01_KnowDesk_PRD.md` chapter 4)
/// that can be measured headlessly. "Search box invocation P95 300ms" and
/// idle CPU/memory require a tray icon, global shortcut, and a resident
/// process to measure, so those are left for Phase C/D.
///
/// Indexing rescans this `path` fresh each time, so for the throughput
/// number to be meaningful, `--db` must point at an empty DB (running
/// against an already-indexed DB mostly just counts SKIPs).
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
        "Indexing: {} total (full-text {} / metadata {} / skipped {}), {elapsed:.2}s ({:.1}/s)",
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
        "Search: {} queries × {repeat} reps\n  P50 {p50:.1}ms / P95 {p95:.1}ms (target within 1000ms — {verdict})",
        queries.len(),
    );

    let db_size = std::fs::metadata(&config.db_path)?.len();
    println!(
        "DB size: {} (source {}, {:.2}x)",
        format_bytes(db_size),
        format_bytes(corpus_bytes),
        db_size as f64 / corpus_bytes.max(1) as f64,
    );

    Ok(())
}

/// Computes the p-th percentile (0-100) from a sorted list of values.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

/// Reads one query per line if the `path` file exists; otherwise uses the
/// default query set.
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

/// The total size of every file under `root` — no index-target filtering
/// (excluding temp files/extensions, etc.) is applied; every file in the
/// folder is summed as-is (the intuitive meaning of "source size").
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
        // Values 1..=100, indices 0..=99. rank = round(p/100 * (len-1)).
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
