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
use knowdesk_cli::parallel_index::{index_directory_parallel, KiwiHandle};
use knowdesk_cli::support::default_extractors_sync;
use knowdesk_core::config::Config;
use knowdesk_core::db::Db;
use knowdesk_core::extract::pdf::PdfExtractor;
use knowdesk_core::nlp::Tokenizer;
use knowdesk_core::search::service::SqliteSearchService;
use knowdesk_core::search::{
    MatchKind, SearchMode as CoreSearchMode, SearchRequest, SearchService,
};
use std::io::IsTerminal;
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
                  Put -f/-l before the query text. Once the query starts, everything \
                  after it — including anything that looks like -f/-l — is part of the \
                  query itself, so a document containing the word \"-l\" is still \
                  findable.\n\n\
                  Filters (x:/p:/m>/m</m=) work the same as the GUI search box - just \
                  type them as part of the query, e.g. `채권 x:pdf p:리서치`."
)]
struct Cli {
    /// Folder to scan.
    path: PathBuf,

    /// Search query — same syntax as the GUI search box (keywords, "phrase",
    /// AND/OR/NOT, prefix*, grouping, and x:/p:/m> filters). Given as separate
    /// words, they're rejoined with spaces, so `AND`/`OR`/`NOT` need no quoting.
    /// Must come after `-f`/`-l` - once it starts, everything remaining
    /// (including a token that looks like `-f`/`-l`) is part of the query
    /// itself, not a flag. This is deliberate: a query genuinely containing a
    /// dash-led word (e.g. searching for the literal text "-l") must still be
    /// possible, and that's only unambiguous if flags are required to come
    /// first.
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

    // A one-shot tool with no idle-CPU budget to protect (unlike the GUI,
    // `docs/06_Development_Roadmap.md` B4) - spend every core to finish
    // faster. `knowdesk_cli::parallel_index` is kdfind-only; `knowdesk-cli`
    // and the GUI keep using `core::index::pipeline::IndexPipeline`'s
    // single-threaded path unchanged (`docs/13_CLI_Tool.md`).
    let kiwi = load_kiwi(&config);

    let db = Db::open_in_memory()?;
    let extractors = default_extractors_sync();
    let index_config = Config::default();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let (conn, _outcome) = index_directory_parallel(
        &cli.path,
        &index_config,
        &extractors,
        kiwi.clone(),
        db.conn,
        threads,
    );

    warn_if_query_contains_flag_like_tokens(&cli.query);

    let service = SqliteSearchService {
        conn: &conn,
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

    let colors = Colors::detect();
    let count = result.hits.len();
    println!(
        "{b}{count} result{s}{r}",
        b = colors.bold,
        s = if count == 1 { "" } else { "s" },
        r = colors.reset,
    );

    for (i, hit) in result.hits.into_iter().enumerate() {
        let (tag, tag_color) = match hit.match_kind {
            MatchKind::Exact => ("exact match", colors.green),
            MatchKind::Morphological => ("morphological match", colors.yellow),
        };
        println!(
            "\n{dim}{n}.{r} {path_c}{path}{r} {tag_c}[{tag}]{r}",
            dim = colors.dim,
            n = i + 1,
            r = colors.reset,
            path_c = colors.path,
            path = hit.path,
            tag_c = tag_color,
        );
        if let Some(snippet) = hit.snippet {
            println!("   {}", colors.highlight(&flatten_snippet(&snippet)));
        }
    }
    Ok(())
}

/// Since flags must precede the query (see `Cli::query`'s doc comment), a
/// flag-looking word placed after the query silently becomes part of the
/// search text instead of being recognized as a flag - e.g. `kdfind ./docs
/// 채권 -l 3` searches for the literal words "채권 -l 3" and (almost always)
/// finds nothing, with no error to explain why. Warn when that's plausibly
/// what happened, without changing behavior - the query is still searched for
/// literally either way, so a document that genuinely contains "-l" as text
/// is still findable, just with this notice alongside it.
fn warn_if_query_contains_flag_like_tokens(query: &[String]) {
    const FLAG_LIKE: &[&str] = &["-f", "--filename", "-l", "--limit"];
    let found: Vec<&str> = query
        .iter()
        .map(String::as_str)
        .filter(|t| FLAG_LIKE.contains(t))
        .collect();
    if !found.is_empty() {
        eprintln!(
            "Notice: \"{}\" in the query looks like a flag, but flags only work before \
             the query text — treating it as a literal search word instead. Move it \
             earlier if you meant the flag, e.g. `kdfind <path> -l 3 <query>`.",
            found.join("\", \"")
        );
    }
}

/// Collapses a possibly multi-line snippet (e.g. an XLSX where cell text is
/// joined with newlines, or a PDF page break falling inside the context
/// window) into a single line, trimming each original line's surrounding
/// whitespace. Otherwise a line break partway through loses this line's
/// indentation on the next print, and the snippet looks pasted in raw rather
/// than formatted.
fn flatten_snippet(snippet: &str) -> String {
    snippet
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Terminal styling for search results - real ANSI highlighting when stdout is
/// an interactive terminal, and a no-op (every field empty) otherwise, so
/// piping to a file or another program still gets plain text. Also off when
/// `NO_COLOR` is set (https://no-color.org).
struct Colors {
    bold: &'static str,
    dim: &'static str,
    path: &'static str,
    green: &'static str,
    yellow: &'static str,
    match_hl: &'static str,
    reset: &'static str,
}

impl Colors {
    fn detect() -> Self {
        let enabled = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        if enabled {
            Self {
                bold: "\x1b[1m",
                dim: "\x1b[2m",
                path: "\x1b[1;36m",
                green: "\x1b[2;32m",
                yellow: "\x1b[2;33m",
                match_hl: "\x1b[1;31m",
                reset: "\x1b[0m",
            }
        } else {
            Self {
                bold: "",
                dim: "",
                path: "",
                green: "",
                yellow: "",
                match_hl: "",
                reset: "",
            }
        }
    }

    /// Replaces `search::service`'s `>>`/`<<` highlight markers with real ANSI
    /// highlighting. Left untouched (literal arrows, as before) when colors are
    /// disabled, so scripts parsing the old plain-text markers keep working.
    fn highlight(&self, snippet: &str) -> String {
        if self.reset.is_empty() {
            return snippet.to_string();
        }
        let mut out = String::with_capacity(snippet.len());
        let mut chars = snippet.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '>' && chars.peek() == Some(&'>') {
                chars.next();
                out.push_str(self.match_hl);
            } else if c == '<' && chars.peek() == Some(&'<') {
                chars.next();
                out.push_str(self.reset);
            } else {
                out.push(c);
            }
        }
        out
    }
}

/// Loads Kiwi strictly from `settings_cli.json` - never from
/// `KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR` (unlike `knowdesk-cli`'s
/// `load_kiwi`). Silent (no notice printed) when morphological analysis is simply
/// off or unconfigured, since that's this tool's default, expected state - only an
/// actual load failure while it's configured on is worth a warning. Spawns Kiwi
/// onto the dedicated actor thread it lives on for the rest of the run
/// (`KiwiHandle::spawn` - `kiwi_rs::Kiwi` isn't `Send`, so it has to be built
/// on that thread directly rather than constructed here and moved over).
fn load_kiwi(config: &CliConfig) -> Option<KiwiHandle> {
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

    match KiwiHandle::spawn(lib_path, model_dir) {
        Ok(handle) => {
            tracing::info!("Using Kiwi morphological analyzer");
            Some(handle)
        }
        Err(e) => {
            eprintln!("Warning: Kiwi initialization failed ({e}) — using bigram only.");
            None
        }
    }
}
