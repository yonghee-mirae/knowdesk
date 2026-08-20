//! `BigramTokenizer` (MVP 초기 구현). 공백으로 구분한 어절마다 2글자 슬라이딩
//! 윈도우를 생성한다. 형태소 분석 없이도 `발행절차` 안의 `발행`을 부분적으로
//! 재현할 수 있게 하는 폴백이며, Kiwi 연동 후 교체된다.

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
