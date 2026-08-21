//! 벤치마크(`cli bench`, Phase B5)용 대량 코퍼스 생성기.
//!
//! 실행:
//! ```text
//! cargo run -p knowdesk-core --example gen_bench_corpus [출력 경로, 기본값 ./bench_corpus] [건수, 기본값 5000]
//! ```
//!
//! `gen_samples`는 포맷별 기능 검증(정상 케이스 + 제외 규칙)이 목적이라 파일 10여 개뿐이다.
//! 벤치마크는 개수·총 용량 규모가 목적이라 그것과는 별도로 둔다. `.txt`만 생성하는데,
//! 포맷별 추출 정확성은 이미 `gen_samples`/익스트랙터 테스트가 커버하고 있어서 여기서는
//! 색인 처리량·검색 P95·DB 크기를 재는 데 필요한 "많은 문서" 자체에만 집중한다.
//!
//! 매 실행마다 내용이 달라지도록 (새 의존성 없이) 직접 만든 xorshift64 PRNG로 문장을
//! 무작위 조합한다 - 결과를 특정 실행에 맞춰 재현하려는 목적이 아니라, 벤치마크 숫자가
//! 우연히 특정 내용에 최적화되어 보이는 착시를 피하려는 것이다.

use std::fs;
use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| "./bench_corpus".to_string());
    let count: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(5000);

    let out_dir = Path::new(&out_dir);
    fs::create_dir_all(out_dir).expect("출력 폴더 생성 실패");

    let mut rng = Rng::seeded();
    for i in 0..count {
        let text = random_document(&mut rng);
        fs::write(out_dir.join(format!("문서_{i:05}.txt")), text).expect("파일 쓰기 실패");
    }

    println!(
        "벤치마크 코퍼스 생성 완료: {count}건, {}",
        out_dir.display()
    );
    println!();
    println!("검증 방법:");
    println!(
        "  cargo run -p knowdesk-cli -- --db ./bench.db bench {}",
        out_dir.display()
    );
}

/// 문서 안에서 조합할 문장 풀. 실제 `cli bench` 기본 검색어(`cli/src/main.rs`의
/// `DEFAULT_QUERIES`)가 여기 등장하는 단어(채권/발행/이사회/결의/국채 등)로 검색했을 때
/// 실제로 매칭되도록 어휘를 맞춰둔다 — 벤치가 "결과 없음"만 반복하며 조기 종료 경로만
/// 재는 것을 방지한다.
const SENTENCES: &[&str] = &[
    "본 문서는 채권 발행 절차를 규정한다.",
    "채권 발행 시 이사회 승인이 필요하다.",
    "이사회는 신규 투자안을 결의했다.",
    "이사회 결의를 통해 채권 발행 계획을 승인한다.",
    "다음 분기 예산안을 검토한다.",
    "국채 금리 변동에 따른 대응 방안을 논의했다.",
    "회의록: 채권 발행 일정을 다음 분기로 연기한다.",
    "발행 규모 및 세부 일정은 별도 공고한다.",
    "이번 분기 실적 보고서를 이사회에 제출한다.",
    "투자 계획은 예산 승인 이후 집행한다.",
    "검토 의견을 반영해 계획을 수정했다.",
    "회사채 발행 조건은 시장 상황에 따라 조정된다.",
    "예산 집행 실적을 분기별로 점검한다.",
    "신규 사업 계획을 이사회에 보고했다.",
];

/// 새 의존성을 추가하지 않기 위해 직접 구현한 최소한의 PRNG (xorshift64).
/// 암호적으로 안전할 필요는 없다 — 매 실행 다른 내용을 만드는 용도뿐이다.
struct Rng(u64);

impl Rng {
    fn seeded() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        Self(nanos | 1) // 0이면 xorshift가 항상 0만 내놓으므로 최소 1비트 보장
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn range(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// 짧은/중간/긴 문서를 섞어서 총 용량 분포가 실제 문서함과 비슷해지게 한다.
fn random_document(rng: &mut Rng) -> String {
    let sentence_count = match rng.range(3) {
        0 => 3 + rng.range(5),    // 짧은 문서: 3~7문장
        1 => 10 + rng.range(20),  // 중간 문서: 10~29문장
        _ => 50 + rng.range(100), // 긴 문서: 50~149문장
    };
    let mut text = String::new();
    for _ in 0..sentence_count {
        text.push_str(SENTENCES[rng.range(SENTENCES.len())]);
        text.push('\n');
    }
    text
}
