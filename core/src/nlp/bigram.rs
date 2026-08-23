//! `BigramTokenizer` (initial MVP implementation). Generates a 2-character sliding
//! window for each whitespace-separated word segment. It's a fallback that lets
//! `발행` inside `발행절차` be partially recovered without morphological analysis,
//! and is replaced once Kiwi is integrated. Scoped to non-ASCII runs within
//! each word (`bigrams_of_word`) — an ASCII letter/digit run (an English
//! word or acronym, however short, and regardless of what punctuation or
//! script it's glued to) is kept as a single token instead, since English
//! has no equivalent of Korean's glued-compound problem this windowing
//! exists for, and windowing it anyway produces spurious matches (e.g. "IAM"
//! -> "IA"/"AM", or "diagram?" -> ...`"ra"`, `"am"`, `"m?"` - matching an
//! unrelated query for "am" either way).

use super::{Token, Tokenizer};

pub struct BigramTokenizer;

impl Tokenizer for BigramTokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token> {
        text.split_whitespace()
            .flat_map(bigrams_of_word)
            .map(Token)
            .collect()
    }
}

/// Splits `word` into maximal runs of ASCII letters/digits vs. everything
/// else (Korean text, punctuation, symbols), then windows only the
/// non-ASCII runs. An earlier version of this function only skipped
/// windowing when the *entire* word was pure ASCII — which missed the very
/// common case of an English word with attached punctuation ("diagram?",
/// "Lambda,", a sentence-ending period, ...): the trailing punctuation made
/// the whole-word check fail, so it still got windowed the old way and could
/// still produce a spurious fragment match (confirmed in practice —
/// "diagram?" windowed into "ra"/"am"/"m?", and "am" then matched an
/// unrelated query). Splitting into runs first means the ASCII word itself
/// is still kept whole regardless of what's attached to it. As a side
/// effect, an English word glued directly to a Korean particle/ending with
/// no space ("TEST하는", common in Korean technical writing) now also yields
/// a clean "TEST" token instead of never producing one at all.
fn bigrams_of_word(word: &str) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let is_ascii_run = chars[i].is_ascii_alphanumeric();
        let start = i;
        while i < chars.len() && chars[i].is_ascii_alphanumeric() == is_ascii_run {
            i += 1;
        }
        let run = &chars[start..i];
        if is_ascii_run || run.len() <= 1 {
            tokens.push(run.iter().collect());
        } else {
            tokens.extend(run.windows(2).map(|w| w.iter().collect::<String>()));
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_into_bigrams() {
        let tokens = BigramTokenizer.tokenize("발행절차");
        let texts: Vec<&str> = tokens.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(texts, vec!["발행", "행절", "절차"]);
    }

    #[test]
    fn keeps_single_char_words() {
        let tokens = BigramTokenizer.tokenize("가 나");
        let texts: Vec<&str> = tokens.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(texts, vec!["가", "나"]);
    }

    #[test]
    fn handles_multiple_words() {
        let tokens = BigramTokenizer.tokenize("채권 발행");
        let texts: Vec<&str> = tokens.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(texts, vec!["채권", "발행"]);
    }

    #[test]
    fn keeps_ascii_words_whole_instead_of_windowing_them() {
        // Reported bug: a document only mentioning AWS "IAM" turned up for a
        // search for "am", because windowing "IAM" the same way as Korean
        // ("IA"/"AM") produced a fragment that happens to equal an unrelated
        // query, case-insensitively. English words have no equivalent of
        // Korean's glued stem+particle problem this windowing exists for.
        let tokens = BigramTokenizer.tokenize("AWS IAM policy");
        let texts: Vec<&str> = tokens.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(texts, vec!["AWS", "IAM", "policy"]);
        assert!(!texts.contains(&"AM"), "must not fragment \"IAM\" into \"IA\"/\"AM\"");
    }

    #[test]
    fn keeps_the_ascii_run_whole_even_inside_a_mixed_word() {
        // The Korean run around it still windows as before ("발행절차" ->
        // "발행"/"행절"/"절차"), and the trailing "(" glues onto the tail of
        // that run the same way punctuation always has - but "AWS" itself,
        // an ASCII run, is no longer fragmented into "(A"/"AW"/"WS"/"S)" the
        // way whole-word windowing used to split across the boundary.
        let tokens = BigramTokenizer.tokenize("발행절차(AWS)");
        let texts: Vec<&str> = tokens.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(texts, vec!["발행", "행절", "절차", "차(", "AWS", ")"]);
    }

    #[test]
    fn keeps_an_ascii_word_whole_even_with_attached_punctuation() {
        // Reported bug: "diagram?" (a sentence ending in a question mark)
        // windowed into "di"/"ia"/"ag"/"gr"/"ra"/"am"/"m?" the old way, and
        // "am" matched a completely unrelated query - the trailing "?" made
        // an earlier version's "is the *whole* word pure ASCII" check fail,
        // so it didn't get the same treatment as a bare "IAM"/"AWS".
        let tokens = BigramTokenizer.tokenize("a diagram? or Lambda, then");
        let texts: Vec<&str> = tokens.iter().map(|t| t.0.as_str()).collect();
        assert!(texts.contains(&"diagram"), "tokens: {texts:?}");
        assert!(texts.contains(&"Lambda"), "tokens: {texts:?}");
        assert!(!texts.contains(&"am"), "must not fragment \"diagram?\" into \"am\": {texts:?}");
    }

    #[test]
    fn splits_an_ascii_word_glued_to_a_korean_particle_into_a_clean_token() {
        // "TEST하는" (an English word directly glued to a Korean
        // verb/particle ending, common in Korean technical writing) used to
        // produce no clean "TEST" token at all - windowed the old way it
        // became "TE"/"ES"/"ST"/"T하"/"하는". Splitting by run means the
        // ASCII portion is recovered as its own token even without Kiwi.
        let tokens = BigramTokenizer.tokenize("TEST하는");
        let texts: Vec<&str> = tokens.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(texts, vec!["TEST", "하는"]);
    }
}
