//! Bulk corpus generator for benchmarking (`cli bench`, Phase B5).
//!
//! Usage:
//! ```text
//! cargo run -p knowdesk-core --example gen_bench_corpus [output path, default ./bench_corpus] [count, default 5000]
//! ```
//!
//! `gen_samples` aims to verify per-format functionality (normal cases + exclusion rules),
//! so it only has a dozen or so files. This benchmark exists separately because its goal is
//! sheer count and total volume. It only generates `.txt` files, since per-format extraction
//! accuracy is already covered by `gen_samples`/extractor tests — here we focus purely on
//! producing the "many documents" needed to measure indexing throughput, search P95, and DB size.
//!
//! Sentences are randomly combined using a hand-rolled xorshift64 PRNG (no new dependency)
//! so the content differs on every run — not to make results reproducible for a specific run,
//! but to avoid the illusion that benchmark numbers happen to be optimized for particular content.

use std::fs;
use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| "./bench_corpus".to_string());
    let count: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(5000);

    let out_dir = Path::new(&out_dir);
    fs::create_dir_all(out_dir).expect("failed to create output folder");

    let mut rng = Rng::seeded();
    for i in 0..count {
        let text = random_document(&mut rng);
        fs::write(out_dir.join(format!("문서_{i:05}.txt")), text).expect("failed to write file");
    }

    println!(
        "Benchmark corpus generation complete: {count} files, {}",
        out_dir.display()
    );
    println!();
    println!("How to verify:");
    println!(
        "  cargo run -p knowdesk-cli -- --db ./bench.db bench {}",
        out_dir.display()
    );
}

/// Pool of sentences to combine inside documents. The vocabulary is chosen so that the
/// `cli bench` default queries (`DEFAULT_QUERIES` in `cli/src/main.rs`) actually match when
/// searching for the words that appear here (채권/발행/이사회/결의/국채, etc.) — this prevents
/// the benchmark from only measuring the early-exit "no results" path over and over.
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

/// Minimal hand-rolled PRNG (xorshift64), implemented to avoid adding a new dependency.
/// It doesn't need to be cryptographically secure — it's only used to vary content per run.
struct Rng(u64);

impl Rng {
    fn seeded() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        Self(nanos | 1) // guarantee at least 1 bit is set, since xorshift with 0 always yields 0
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

/// Mix short/medium/long documents so the total size distribution resembles a real document folder.
fn random_document(rng: &mut Rng) -> String {
    let sentence_count = match rng.range(3) {
        0 => 3 + rng.range(5),    // short document: 3-7 sentences
        1 => 10 + rng.range(20),  // medium document: 10-29 sentences
        _ => 50 + rng.range(100), // long document: 50-149 sentences
    };
    let mut text = String::new();
    for _ in 0..sentence_count {
        text.push_str(SENTENCES[rng.range(SENTENCES.len())]);
        text.push('\n');
    }
    text
}
