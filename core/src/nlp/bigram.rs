//! `BigramTokenizer` (initial MVP implementation). Generates a 2-character sliding
//! window for each whitespace-separated word segment. It's a fallback that lets
//! `발행` inside `발행절차` be partially recovered without morphological analysis,
//! and is replaced once Kiwi is integrated.

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

fn bigrams_of_word(word: &str) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    if chars.len() <= 1 {
        return vec![word.to_string()];
    }
    chars.windows(2).map(|w| w.iter().collect()).collect()
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
}
