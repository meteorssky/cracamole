//! Deterministic structure-aware fuzzing (run with `cargo test --release -- --ignored`).
//! A stand-in for cargo-fuzz on machines without a nightly toolchain.

use guac_protocol::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn iters() -> usize {
    std::env::var("STRESS_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300_000)
}

fn valid_stream(rng: &mut Rng) -> Vec<u8> {
    let mut out = vec![];
    for _ in 0..rng.below(6) + 1 {
        let n = rng.below(5);
        let args: Vec<String> = (0..n)
            .map(|_| {
                let len = rng.below(12);
                (0..len)
                    .map(|_| match rng.below(6) {
                        0 => '中',
                        1 => '😀',
                        2 => ',',
                        3 => ';',
                        4 => '.',
                        _ => (b'a' + rng.below(26) as u8) as char,
                    })
                    .collect()
            })
            .collect();
        Instruction::new("op", args).encode_into(&mut out);
    }
    out
}

#[test]
#[ignore]
fn stress_parser_mutated_streams_never_panic_and_stay_bounded() {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let mut parsed = 0usize;
    for _ in 0..iters() {
        let mut data = valid_stream(&mut rng);
        // mutate: overwrite, insert, delete, truncate, duplicate
        for _ in 0..rng.below(4) {
            if data.is_empty() {
                break;
            }
            match rng.below(5) {
                0 => {
                    let i = rng.below(data.len());
                    data[i] = rng.next() as u8;
                }
                1 => {
                    let i = rng.below(data.len());
                    data.insert(i, rng.next() as u8);
                }
                2 => {
                    let i = rng.below(data.len());
                    data.remove(i);
                }
                3 => data.truncate(rng.below(data.len()) + 1),
                _ => {
                    let c = data.clone();
                    data.extend(c);
                }
            }
        }
        let mut p = Parser::new();
        let mut pos = 0;
        while pos < data.len() {
            let n = (rng.below(9) + 1).min(data.len() - pos);
            if p.feed(&data[pos..pos + n]).is_err() {
                break;
            }
            pos += n;
        }
        while let Some(i) = p.next_instruction() {
            parsed += 1;
            assert!(i.args.len() <= MAX_ARGS);
            assert!(i.opcode.chars().count() <= MAX_ELEMENT_LEN);
            assert!(i.args.iter().all(|a| a.chars().count() <= MAX_ELEMENT_LEN));
            // anything the parser accepts must re-encode and re-parse identically
            let mut q = Parser::new();
            q.feed(&i.encode()).unwrap();
            assert_eq!(q.next_instruction().unwrap(), i);
        }
    }
    eprintln!(
        "stress parser: {} iterations, {} instructions parsed",
        iters(),
        parsed
    );
    assert!(
        parsed > iters() / 4,
        "generator should produce mostly parseable data"
    );
}

#[test]
#[ignore]
fn stress_parser_pure_noise() {
    let mut rng = Rng(42);
    for _ in 0..iters() {
        let n = rng.below(300);
        let data: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
        let mut p = Parser::new();
        let _ = p.feed(&data);
        while p.next_instruction().is_some() {}
    }
}
