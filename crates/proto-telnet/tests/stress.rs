use proto_telnet::codec::*;
use proto_telnet::negotiation::Negotiator;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
#[ignore]
fn stress_telnet_decoder_and_negotiator_on_hostile_streams() {
    let n: usize = std::env::var("STRESS_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);
    let mut rng = Rng(7);
    let mut neg = Negotiator::new("linux", Some("u"), 80, 24);
    let mut dec = Decoder::new();
    let mut replies = 0usize;
    for _ in 0..n {
        // IAC-heavy random bytes
        let len = (rng.next() % 64) as usize;
        let data: Vec<u8> = (0..len)
            .map(|_| match rng.next() % 4 {
                0 => IAC,
                1 => [DO, DONT, WILL, WONT, SB, SE][(rng.next() % 6) as usize],
                _ => rng.next() as u8,
            })
            .collect();
        for ev in dec.decode(&data) {
            let r = neg.handle(ev);
            replies += r.send.len();
        }
    }
    // Replies must stay bounded: repeated identical requests are answered at most once per option.
    assert!(
        replies < n * 40,
        "negotiation reply volume exploded: {replies}"
    );
}
