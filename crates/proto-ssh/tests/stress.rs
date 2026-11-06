mod common;

use proto_ssh::{hostkey, keys};

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

#[test]
#[ignore]
fn stress_known_hosts_parser_and_matcher() {
    let n: usize = std::env::var("STRESS_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);
    let mut rng = Rng(5);
    let alphabet = b"|1|,!*?[]:.-_ abcdefxyz0123456789+/=\n#@";
    for _ in 0..n {
        let len = rng.below(120);
        let s: String = (0..len)
            .map(|_| alphabet[rng.below(alphabet.len())] as char)
            .collect();
        let (entries, _) = hostkey::parse(&s);
        let _ = hostkey::verify(&entries, "host.example", 22, &[1, 2, 3]);
        let _ = hostkey::verify(&entries, "h", 2222, &[]);
    }
}

#[test]
#[ignore]
fn stress_private_key_loader_on_mutated_keys() {
    let Some((pem, _)) = common::keygen("ed25519", None, "") else {
        eprintln!("SKIP: ssh-keygen unavailable");
        return;
    };
    let mut rng = Rng(11);
    let bytes = pem.into_bytes();
    for _ in 0..2_000 {
        let mut b = bytes.clone();
        for _ in 0..rng.below(6) + 1 {
            if b.is_empty() {
                break;
            }
            let i = rng.below(b.len());
            match rng.below(3) {
                0 => b[i] = rng.next() as u8,
                1 => {
                    b.remove(i);
                }
                _ => b.truncate(i + 1),
            }
        }
        let text = String::from_utf8_lossy(&b).into_owned();
        let _ = keys::load_private_key(&text, None);
        let _ = keys::load_private_key(&text, Some("x"));
    }
}
