//! `KiwiTokenizer` (Phase B2). `BigramTokenizer`가 항상 채우는 기본 토크나이저이고,
//! 이 토크나이저는 가능할 때만 추가로 붙는 보조 토크나이저다 — 색인의 `morph_kiwi`
//! 컬럼과 검색어 확장(`search::service`) 양쪽에서 같은 방식으로 쓰인다.
//!
//! 오프라인 초기화만 사용한다 — `Kiwi::init()`은 GitHub 릴리스에서 라이브러리·모델을
//! 자동 다운로드하므로 망분리 환경에서 쓸 수 없다 (`11_Implementation_Plan.md`
//! "Kiwi 오프라인 초기화" 참조). 대신 `Kiwi::from_config`로 동봉 경로를 명시한다.
//!
//! 라이브러리/모델 경로는 `KNOWDESK_KIWI_LIB_PATH`(네이티브 라이브러리 파일 경로)와
//! `KNOWDESK_KIWI_MODEL_DIR`(모델 디렉터리 경로) 환경 변수로 지정한다. 둘 중 하나라도
//! 없으면 초기화를 시도하지 않는다 — bigram만 쓰는 건 호출부(`cli`)의 책임이다.
//!
//! `tokenize()`는 조사/어미/구두점 등 순수 문법 형태소를 제외하고 의미 형태소(명사/
//! 동사·형용사 어간/부사/관형사 등)만 반환한다. 색인 시점과 검색어 확장 시점에
//! 똑같이 적용해야, "짓다"처럼 어미가 붙은 사전형 검색어가 의미 있는 어간("짓")만
//! 남아 과도하게 넓은 매칭(예: 흔한 어미 "다")을 만들지 않는다.
//!
//! `locate()`는 불규칙 활용(ㅅ 불규칙 등)으로 표면형과 분석 결과가 달라 원문에서
//! 형태소를 리터럴로 찾을 수 없을 때 쓴다 — "지었다"를 "짓"으로 찾았어도, `짓/VV-I`
//! 토큰의 `position`/`length`(kiwi-rs가 원문 기준 글자 단위로 준다)를 이용해
//! "지었다"라는 원문 구간을 그대로 강조할 수 있다 (`search::service` 참조).

use super::{Token, Tokenizer};
use kiwi_rs::{BuilderConfig, Kiwi, KiwiConfig, KIWI_BUILD_DEFAULT_WITH_CONG};
use std::path::PathBuf;

pub struct KiwiTokenizer {
    kiwi: Kiwi,
}

impl KiwiTokenizer {
    /// `KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR` 환경 변수로 오프라인 초기화한다.
    /// 둘 중 하나라도 설정되어 있지 않으면 `None` — 에러가 아니라 "설정 안 됨"이다.
    pub fn from_env() -> Option<Result<Self, String>> {
        let lib_path = std::env::var("KNOWDESK_KIWI_LIB_PATH").ok()?;
        let model_dir = std::env::var("KNOWDESK_KIWI_MODEL_DIR").ok()?;
        Some(Self::new(lib_path.into(), model_dir.into()))
    }

    pub fn new(lib_path: PathBuf, model_dir: PathBuf) -> Result<Self, String> {
        // 배포용 모델(`kiwi_model_v0.23.2_base.tgz`)은 `models/cong/base`에 CONG 계열
        // 모델(`cong.mdl`)만 담고 있다. 기본 build_options(`KIWI_BUILD_DEFAULT`)는
        // KNLM 계열을 가정해 CONG 모델을 잘못 해석하므로 `KIWI_BUILD_DEFAULT_WITH_CONG`을
        // 명시해야 한다 (`kiwi-cli --model-type cong`와 동등).
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
                tracing::warn!(error = %e, "Kiwi 토큰화 실패, 빈 결과로 처리");
                Vec::new()
            }
        }
    }

    fn locate(&self, text: &str, forms: &[String]) -> Option<(usize, usize)> {
        let tokens = self.kiwi.tokenize(text).ok()?;
        let matched = tokens.iter().find(|t| forms.iter().any(|f| f == &t.form))?;

        // 형태소 하나만 강조하면 어간만 짧게 강조되어 부자연스럽다("지었다"의 "지"만
        // 강조되는 식). 같은 어절(word_position)에 속한 형태소 전체를 모아 어절의
        // 시작~끝 구간으로 넓힌다.
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

/// 세종 품사 태그 중 의미 형태소(체언/용언 어간/부사·관형사/어근 등)만 남긴다.
/// 조사(JK*/JX/JC)·어미(EP/EF/EC/ET*)·접미사(XS*)·구두점(S*)은 제외한다.
fn is_content_tag(tag: &str) -> bool {
    const CONTENT_PREFIXES: &[&str] = &[
        "NNG", "NNP", "NNB", "NR", "NP", // 체언(명사/수사/대명사)
        "VV", "VA", "VX", // 용언 어간(동사/형용사/보조용언)
        "MM", "MAG", "MAJ", // 관형사/부사
        "IC",  // 감탄사
        "XR", "XPN", // 어근/접두사
        "SL", "SH", "SN", // 외국어/한자/숫자
    ];
    CONTENT_PREFIXES
        .iter()
        .any(|prefix| tag.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlp::bigram::BigramTokenizer;

    /// `KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`가 설정된 환경에서만 통과한다.
    /// 네이티브 라이브러리·모델이 없는 환경(CI 등)에서는 건너뛴다.
    #[test]
    fn tokenizes_and_beats_bigram_recall() {
        let Some(result) = KiwiTokenizer::from_env() else {
            eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR 미설정, 건너뜁니다");
            return;
        };
        let tokenizer = result.expect("Kiwi 초기화 실패");

        let text = "채권 발행절차를 이사회에서 승인했다.";
        let tokens = tokenizer.tokenize(text);
        let forms: Vec<&str> = tokens.iter().map(|t| t.0.as_str()).collect();

        // 형태소 분석은 "발행절차"를 "발행"+"절차"로 정확히 분리해야 한다.
        // bigram 폴백은 "행절"처럼 형태소 경계를 넘는 부정확한 조각을 만든다 — 이것이
        // TASK-504(재현율 비교)가 요구하는 차이다.
        assert!(forms.contains(&"발행"), "형태소 토큰: {forms:?}");
        assert!(forms.contains(&"절차"), "형태소 토큰: {forms:?}");

        // 문맥이 있으면 "이사회"가 뒤에 붙은 조사("에서")와 정확히 분리된다.
        assert!(forms.contains(&"이사회"), "형태소 토큰: {forms:?}");

        // 품사 필터: 조사/어미는 제외되고 어간만 남아야 한다.
        assert!(!forms.contains(&"를"), "조사가 남아있음: {forms:?}");
        assert!(!forms.contains(&"에서"), "조사가 남아있음: {forms:?}");
        assert!(!forms.contains(&"다"), "어미가 남아있음: {forms:?}");

        let bigram_forms: Vec<String> = BigramTokenizer
            .tokenize(text)
            .into_iter()
            .map(|t| t.0)
            .collect();
        assert!(
            bigram_forms.contains(&"행절".to_string()),
            "bigram이 형태소 경계를 넘는 조각을 만드는지 확인 (비교 기준선): {bigram_forms:?}"
        );
    }

    #[test]
    fn locates_irregular_verb_surface_span_by_stem() {
        let Some(result) = KiwiTokenizer::from_env() else {
            eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR 미설정, 건너뜁니다");
            return;
        };
        let tokenizer = result.expect("Kiwi 초기화 실패");

        let text = "그는 새 건물을 지었다.";
        let (start, len) = tokenizer
            .locate(text, &["짓".to_string()])
            .expect("어간 위치를 찾아야 한다");

        let chars: Vec<char> = text.chars().collect();
        let span: String = chars[start..start + len].iter().collect();
        // 마침표(SF)가 "다"와 같은 word_position으로 묶여 "지었다."까지 포함된다 —
        // 문제 없다, 강조 범위가 문장부호까지 살짝 넓어질 뿐 표시에는 문제없다.
        assert_eq!(span, "지었다.", "찾은 구간: {span:?}");
    }
}
