//! `kdfind` - a search tool that doesn't need a pre-built index DB
//! (`docs/13_CLI_Tool.md`). Scans a folder into an in-memory index and searches
//! it in the same run, then discards everything - nothing is left on disk.
//!
//! Distributed standalone (separately from the GUI app and `knowdesk-cli`), so
//! unlike both of those, it reads no `KNOWDESK_*` environment variables at all -
//! every native library path comes from its own `settings_cli.json`
//! (`knowdesk_cli::cli_config`).

use clap::Parser;
use knowdesk_cli::cli_config::{cli_settings_path, CliConfig};
use knowdesk_cli::support::default_extractors;
use knowdesk_core::config::Config;
use knowdesk_core::db::Db;
use knowdesk_core::extract::pdf::PdfExtractor;
use knowdesk_core::index::pipeline::IndexPipeline;
use knowdesk_core::nlp::bigram::BigramTokenizer;
use knowdesk_core::nlp::kiwi::KiwiTokenizer;
use knowdesk_core::nlp::Tokenizer;
use knowdesk_core::search::service::SqliteSearchService;
use knowdesk_core::search::{
    MatchKind, SearchMode as CoreSearchMode, SearchRequest, SearchService,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "kdfind",
    about = "Searches a folder without a pre-built index — scans it in-memory and \
             searches it in the same run, then discards everything.",
    after_help = "EXAMPLES:\n  \
                  kdfind ./docs 채권 AND 발행          # no quoting needed - AND/OR/NOT \
                  are plain words\n  \
                  kdfind ./docs '\"채권 발행\"'          # phrase search: wrap the whole \
                  thing in single quotes so the shell\n                                       \
                  passes the literal double quotes through\n  \
                  kdfind ./docs -f -l 5 보고서          # filename mode, top 5 results\n\n\
                  Put -f/-l before the query text - once the query starts, every \
                  remaining argument (including a later -f/-l) becomes part of it.\n\n\
                  Filters (x:/p:/m>/m</m=) work the same as the GUI search box - just \
                  type them as part of the query, e.g. `채권 x:pdf p:리서치`."
)]
struct Cli {
    /// Folder to scan.
    path: PathBuf,

    /// Search query — same syntax as the GUI search box (keywords, "phrase",
    /// AND/OR/NOT, prefix*, grouping, and x:/p:/m> filters). Given as separate
    /// words, they're rejoined with spaces, so `AND`/`OR`/`NOT` need no quoting.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    query: Vec<String>,

    /// Filename-mode search instead of the default content search.
    #[arg(short = 'f', long = "filename")]
    filename: bool,

    /// Max number of results. 0 = unlimited.
    #[arg(short = 'l', long = "limit", default_value_t = 0)]
    limit: i64,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    let settings_path = cli_settings_path();
    let config = if settings_path.exists() {
        CliConfig::load(&settings_path)?
    } else {
        // First run - write the defaults out so there's a file the user can find
        // and edit (same convention as the GUI's settings.json).
        let config = CliConfig::default();
        if let Err(e) = config.save(&settings_path) {
            eprintln!("Warning: failed to write {}: {e}", settings_path.display());
        }
        config
    };

    // kdfind resolves every native library path from `settings_cli.json` alone -
    // `PdfExtractor` otherwise falls back to `KNOWDESK_PDFIUM_LIB_DIR`, which this
    // tool deliberately never reads.
    PdfExtractor::set_lib_dir(config.pdfium_lib_dir.clone());

    let kiwi = load_kiwi(&config);

    let db = Db::open_in_memory()?;
    let extractors = default_extractors();
    let bigram = BigramTokenizer;
    let index_config = Config::default();
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &index_config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: kiwi.as_ref().map(|k| k as &dyn Tokenizer),
    };
    pipeline.index_directory(&cli.path)?;

    let service = SqliteSearchService {
        conn: &db.conn,
        kiwi: kiwi.as_ref().map(|k| k as &dyn Tokenizer),
    };
    let request = SearchRequest {
        query: cli.query.join(" "),
        mode: if cli.filename {
            CoreSearchMode::Filename
        } else {
            CoreSearchMode::Content
        },
        limit: cli.limit,
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

/// Loads Kiwi strictly from `settings_cli.json` - never from
/// `KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR` (unlike `knowdesk-cli`'s
/// `load_kiwi`). Silent (no notice printed) when morphological analysis is simply
/// off or unconfigured, since that's this tool's default, expected state - only an
/// actual load failure while it's configured on is worth a warning.
fn load_kiwi(config: &CliConfig) -> Option<KiwiTokenizer> {
    if !config.enable_morphological_analysis {
        return None;
    }
    let (Some(lib_path), Some(model_dir)) =
        (config.kiwi_lib_path.clone(), config.kiwi_model_dir.clone())
    else {
        eprintln!(
            "Warning: enable_morphological_analysis is on but kiwi_lib_path/kiwi_model_dir \
             are missing in settings_cli.json — using bigram only."
        );
        return None;
    };

    match KiwiTokenizer::new(lib_path, model_dir) {
        Ok(kiwi) => {
            tracing::info!("Using Kiwi morphological analyzer");
            Some(kiwi)
        }
        Err(e) => {
            eprintln!("Warning: Kiwi initialization failed ({e}) — using bigram only.");
            None
        }
    }
}
