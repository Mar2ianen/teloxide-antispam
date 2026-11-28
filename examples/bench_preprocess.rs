use std::{hint::black_box, time::Instant};

use teloxide_antispam::preprocess::{PreparedText, prepare_text, prepare_text_v3};

fn measure(label: &str, text: &str, prepare: fn(&str) -> PreparedText) {
    let iterations = ((2 * 1024 * 1024) / text.len().max(1)).max(1);
    let mut samples = Vec::with_capacity(19);
    for _ in 0..19 {
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(prepare(black_box(text)));
        }
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let median = samples[9];
    let throughput =
        text.len() as f64 * iterations as f64 / median.as_secs_f64() / (1024.0 * 1024.0);
    println!(
        "{label}: {} bytes, {iterations} iterations, median {} ns, {:.2} MiB/s",
        text.len(),
        median.as_nanos(),
        throughput,
    );
}

fn compare(label: &str, text: &str) {
    measure(&format!("{label} V2"), text, prepare_text);
    measure(&format!("{label} V3"), text, prepare_text_v3);
}

fn main() {
    compare("ASCII", &"Hello world! Offer details here. ".repeat(248));
    compare("Cyrillic", &"Привет, мир! Есть предложение. ".repeat(520));
    compare(
        "Mixed Unicode",
        &"Ищeшь dоход? Café, ＤＤＲ５. ".repeat(393),
    );
    compare("Emoji-heavy", &"👩‍💻 ❤️ 🏳️‍🌈 👨‍👩‍👧‍👦 👍🏽 😂 ".repeat(249));
    compare("Indic adversarial", &"क्‍षि".repeat(69_905));
    compare(
        "Rich-like max",
        &"Visit https://example.org/👩‍💻?q=1 — Открыть таблицу · value\n".repeat(12_336),
    );
}
