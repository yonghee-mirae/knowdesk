//! `KiwiTokenizer` (Phase B2). `BigramTokenizer` is the primary tokenizer that always
//! runs, and this tokenizer is a secondary one that's added on top when available — it's
//! used the same way for both the index's `morph_kiwi` column and query expansion
//! (`search::service`).
//!
//! Only offline initialization is used — `Kiwi::init()` auto-downloads the library/model
//! from a GitHub release, so it can't be used in an air-gapped environment (see
//! "Kiwi offline initialization" in `11_Implementation_Plan.md`). Instead, the bundled
//! paths are specified explicitly via `Kiwi::from_config`.
//!
//! The library/model paths are given via the `KNOWDESK_KIWI_LIB_PATH` (native library
//! file path) and `KNOWDESK_KIWI_MODEL_DIR` (model directory path) environment
//! variables. If either is missing, initialization isn't attempted — it's the caller's
//! (`cli`) responsibility to fall back to bigram only.
//!
//! `tokenize()` excludes purely grammatical morphemes such as particles/endings/
//! punctuation, and returns only content morphemes (noun/verb·adjective stems/adverbs/
//! determiners, etc.). This must be applied identically at index time and at query
//! expansion time, so that a dictionary-form query with an ending attached, like "짓다"
//! (to build), is left with only the meaningful stem ("짓") and doesn't produce overly
//! broad matches (e.g. on the common ending "다").
//!
//! `locate()` is used when irregular conjugation (e.g. the ㅅ-irregular) makes the
//! surface form differ from the analyzed form, so the morpheme can't be found literally
//! in the original text — even though "지었다" (built) was found as "짓" (to build), the
//! `position`/`length` of the `짓/VV-I` token (given by kiwi-rs in character units
//! relative to the original text) can be used to highlight the original "지었다" span as
//! it appears (see `search::service`).

use super::{Token, Tokenizer};
use kiwi_rs::{BuilderConfig, Kiwi, KiwiConfig, KIWI_BUILD_DEFAULT_WITH_CONG};
use std::path::PathBuf;

pub struct KiwiTokenizer {
    kiwi: Kiwi,
}

impl KiwiTokenizer {
    /// Initializes offline using the `KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`
    /// environment variables. If either is not set, returns `None` — meaning "not
    /// configured", not an error.
    pub fn from_env() -> Option<Result<Self, String>> {
        let lib_path = std::env::var("KNOWDESK_KIWI_LIB_PATH").ok()?;
        let model_dir = std::env::var("KNOWDESK_KIWI_MODEL_DIR").ok()?;
        Some(Self::new(lib_path.into(), model_dir.into()))
    }

    pub fn new(lib_path: PathBuf, model_dir: PathBuf) -> Result<Self, String> {
        // The distributed model (`kiwi_model_v0.23.2_base.tgz`) contains only the CONG
        // family model (`cong.mdl`) under `models/cong/base`. The default build_options
        // (`KIWI_BUILD_DEFAULT`) assumes the KNLM family and misinterprets the CONG model,
        // so `KIWI_BUILD_DEFAULT_WITH_CONG` must be specified explicitly (equivalent to
        // `kiwi-cli --model-type cong`).
        let builder = BuilderConfig {
            model_path: Some(model_dir),
            build_options: KIWI_BUILD_DEFAULT_WITH_CONG,
            ..BuilderConfig::default()
        };
        let config = KiwiConfig::default()
            .with_library_path(lib_path)
            .with_builder(builder);
        let kiwi = Kiwi::from_config(config).map_err(|e| e.to_string())?;
        Ok(Self { kiwi })
    }
}

impl Tokenizer for KiwiTokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token> {
        match self.kiwi.tokenize(text) {
            Ok(tokens) => tokens
                .into_iter()
                .filter(|t| is_content_tag(&t.tag))
                .map(|t| Token(t.form))
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "Kiwi tokenization failed, treating as empty result");
                Vec::new()
            }
        }
    }

    fn locate(&self, text: &str, forms: &[String]) -> Option<(usize, usize)> {
        let tokens = self.kiwi.tokenize(text).ok()?;
        let matched = tokens.iter().find(|t| forms.iter().any(|f| f == &t.form))?;

        // Highlighting only a single morpheme looks unnatural, since just the short stem
        // gets highlighted (e.g. only "지" out of "지었다" (built)). Gather all morphemes
        // belonging to the same word segment (word_position) and widen the span to the
        // segment's full start~end range.
        let (start, end) = tokens
            .iter()
            .filter(|t| t.word_position == matched.word_position)
            .fold(
                (matched.position, matched.position + matched.length),
                |(s, e), t| (s.min(t.position), e.max(t.position + t.length)),
            );
        Some((start, end - start))
    }
}

/// Among Sejong part-of-speech tags, keeps only content morphemes (substantives/
/// predicate stems/adverbs·determiners/roots, etc.).
/// Excludes particles (JK*/JX/JC), endings (EP/EF/EC/ET*), suffixes (XS*), and
/// punctuation (S*).
fn is_content_tag(tag: &str) -> bool {
    const CONTENT_PREFIXES: &[&str] = &[
        "NNG", "NNP", "NNB", "NR", "NP", // substantives (nouns/numerals/pronouns)
        "VV", "VA", "VX", // predicate stems (verbs/adjectives/auxiliary predicates)
        "MM", "MAG", "MAJ", // determiners/adverbs
        "IC",  // interjections
        "XR", "XPN", // roots/prefixes
        "SL", "SH", "SN", // foreign words/Hanja/numbers
    ];
    CONTENT_PREFIXES
        .iter()
        .any(|prefix| tag.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlp::bigram::BigramTokenizer;

    /// Passes only in an environment where `KNOWDESK_KIWI_LIB_PATH`/
    /// `KNOWDESK_KIWI_MODEL_DIR` are set.
    /// Skipped in environments without the native library/model (e.g. CI).
    #[test]
    fn tokenizes_and_beats_bigram_recall() {
        let Some(result) = KiwiTokenizer::from_env() else {
            eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR not set, skipping");
            return;
        };
        let tokenizer = result.expect("Kiwi initialization failed");

        let text = "채권 발행절차를 이사회에서 승인했다.";
        let tokens = tokenizer.tokenize(text);
        let forms: Vec<&str> = tokens.iter().map(|t| t.0.as_str()).collect();

        // Morphological analysis must correctly split "발행절차" into "발행"+"절차".
        // The bigram fallback produces inaccurate fragments that cross morpheme
        // boundaries, like "행절" — that's the difference TASK-504 (recall comparison)
        // requires.
        assert!(forms.contains(&"발행"), "morpheme tokens: {forms:?}");
        assert!(forms.contains(&"절차"), "morpheme tokens: {forms:?}");

        // With context, "이사회" (board of directors) is correctly separated from the
        // particle ("에서") attached after it.
        assert!(forms.contains(&"이사회"), "morpheme tokens: {forms:?}");

        // Part-of-speech filter: particles/endings must be excluded, leaving only stems.
        assert!(!forms.contains(&"를"), "particle remains: {forms:?}");
        assert!(!forms.contains(&"에서"), "particle remains: {forms:?}");
        assert!(!forms.contains(&"다"), "ending remains: {forms:?}");

        let bigram_forms: Vec<String> = BigramTokenizer
            .tokenize(text)
            .into_iter()
            .map(|t| t.0)
            .collect();
        assert!(
            bigram_forms.contains(&"행절".to_string()),
            "checking that bigram produces fragments crossing morpheme boundaries (baseline for comparison): {bigram_forms:?}"
        );
    }

    #[test]
    fn locates_irregular_verb_surface_span_by_stem() {
        let Some(result) = KiwiTokenizer::from_env() else {
            eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR not set, skipping");
            return;
        };
        let tokenizer = result.expect("Kiwi initialization failed");

        let text = "그는 새 건물을 지었다.";
        let (start, len) = tokenizer
            .locate(text, &["짓".to_string()])
            .expect("must find the stem position");

        let chars: Vec<char> = text.chars().collect();
        let span: String = chars[start..start + len].iter().collect();
        // The period (SF) is grouped under the same word_position as "다" (an ending),
        // so "지었다." (built.) ends up included — that's fine, the highlighted span
        // just extends slightly to include the punctuation, which doesn't cause display
        // issues.
        assert_eq!(span, "지었다.", "found span: {span:?}");
    }
}
