//! `SynonymDictionary` — synonym dictionary abstraction (Phase B3,
//! `docs/06_Development_Roadmap.md`).
//!
//! What Kiwi connects is only conjugated forms of the same word (e.g. "짓다" (to build)
//! → "지었다" (built), see `KiwiTokenizer`). Words that mean the same thing but are
//! written completely differently (in-house abbreviations, jargon, etc., e.g.
//! "ELS"↔"주가연계증권" (equity-linked security)) can't be connected by morphological
//! analysis, and can only be linked via an explicitly registered dictionary.
//!
//! Per a decision (2026-08-21) that this feature isn't needed right now, only the
//! interface is defined here and the implementation is deferred — `KnowDesk_추가검토사항.md`
//! item D-3 ("in-house abbreviation synonym dictionary — whether to provide user
//! registration") is also still open, so the concrete shape (a read-only file-based
//! dictionary vs. one needing a user-editable UI, etc.) hasn't been decided yet.
//! Once implemented, it plugs in at the same spot as `Tokenizer`, via
//! `Option<&dyn SynonymDictionary>` in `search::service`'s query expansion logic — it
//! would simply be added as a third expansion axis alongside the current setup of
//! bigram (primary) and Kiwi (secondary).

pub trait SynonymDictionary {
    /// Returns the list of synonyms for `term`. An empty vector if none.
    fn synonyms(&self, term: &str) -> Vec<String>;
}
