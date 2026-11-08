//! The decoder reads from an untrusted peer: arbitrary bytes must end in an error or a clean
//! result, never a panic, an overflow, a hang or a huge allocation.

use proto_vnc::decode::Decoder;
use proto_vnc::format::PixelFormat;
use proto_vnc::screen::Screen;
use std::time::Duration;

/// xorshift64*: deterministic, dependency-free pseudo randomness.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

async fn feed(bytes: Vec<u8>, format: PixelFormat) {
    let mut screen = Screen::new(96, 80).unwrap();
    let mut decoder = Decoder::new(format);
    let mut reader = &bytes[..];
    // Each call consumes input or fails; the loop ends when the data runs out.
    let run = async {
        for _ in 0..64 {
            if decoder
                .read_message(&mut reader, &mut screen)
                .await
                .is_err()
            {
                break;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("decoder must not hang");
}

/// A plausible update header followed by noise, so that decoding gets past the first checks.
fn structured(rng: &mut Rng) -> Vec<u8> {
    let encodings: [i32; 8] = [0, 1, 5, 16, -223, -308, 2, 99];
    let mut data = vec![];
    for _ in 0..rng.below(4) + 1 {
        data.extend_from_slice(&[0, 0, 0, (rng.below(3) + 1) as u8]);
        for _ in 0..3 {
            let (x, y) = (rng.below(100) as u16, rng.below(90) as u16);
            let (w, h) = (rng.below(40) as u16, rng.below(40) as u16);
            for v in [x, y, w, h] {
                data.extend_from_slice(&v.to_be_bytes());
            }
            data.extend_from_slice(&encodings[rng.below(8) as usize].to_be_bytes());
            let n = rng.below(300) as usize;
            data.extend((0..n).map(|_| rng.next() as u8));
        }
    }
    data
}

#[tokio::test]
async fn random_streams_never_panic_or_hang() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for format in [
        PixelFormat::rgb32(false),
        PixelFormat::rgb565(false),
        PixelFormat::bgr233(true),
    ] {
        for round in 0..400 {
            let data = if round % 2 == 0 {
                let n = rng.below(600) as usize;
                (0..n).map(|_| rng.next() as u8).collect()
            } else {
                structured(&mut rng)
            };
            feed(data, format).await;
        }
    }
}

#[tokio::test]
async fn huge_declared_lengths_are_not_allocated_up_front() {
    // ZRLE rectangle claiming 4 GiB of data, then nothing: must fail quickly.
    let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0, 0, 8, 0, 8, 0, 0, 0, 16];
    data.extend_from_slice(&u32::MAX.to_be_bytes());
    feed(data, PixelFormat::rgb32(false)).await;
    // Cut text claiming 4 GiB.
    let mut data = vec![3, 0, 0, 0];
    data.extend_from_slice(&u32::MAX.to_be_bytes());
    feed(data, PixelFormat::rgb32(false)).await;
    // Colour map claiming 65535 entries with no data.
    feed(vec![1, 0, 0, 0, 0xff, 0xff], PixelFormat::rgb32(false)).await;
}
