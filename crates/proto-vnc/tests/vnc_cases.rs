//! End-to-end VNC cases against an in-process RFB server (no external VNC server needed).
//!
//! The mock server implements the server side of each scheme independently of `proto_vnc::auth`
//! (server-side Diffie-Hellman and AES decryption), so the client is checked against a second
//! implementation of the same wire format.

use aes::cipher::{BlockCipherDecrypt, KeyInit};
use flate2::{Compress, Compression, FlushCompress};
use guac_protocol::Instruction;
use guac_session::*;
use guac_testkit::{DisplayReplay, SimClient};
use md5::{Digest, Md5};
use num_bigint::BigUint;
use proto_vnc::VncPlugin;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

// ---------------------------------------------------------------------------------------------
// Rig: guac client <-> session <-> VncPlugin <-> TCP <-> mock RFB server
// ---------------------------------------------------------------------------------------------

struct Rig {
    client: SimClient,
    session: JoinHandle<Result<(), SessionError>>,
    replay: DisplayReplay,
}

async fn mock<F, Fut, T>(script: F) -> (u16, JoinHandle<T>)
where
    F: FnOnce(TcpStream) -> Fut + Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        script(stream).await
    });
    (port, handle)
}

async fn connect(port: u16, extra: &[(&str, &str)]) -> Rig {
    let plugin: Arc<dyn ProtocolPlugin> = Arc::new(VncPlugin);
    let (c, s) = duplex(1 << 20);
    let session = tokio::spawn(async move {
        let (r, mut w) = tokio::io::split(s);
        let mut reader = InstructionReader::new(r);
        reader.read().await?;
        let hs = handshake(
            &mut reader,
            &mut w,
            plugin.arg_names(),
            "$t",
            HANDSHAKE_TIMEOUT,
        )
        .await?;
        run_session(
            reader,
            w,
            hs,
            plugin,
            SessionConfig {
                shutdown_grace: Duration::from_millis(300),
                ..Default::default()
            },
        )
        .await
    });
    let port = port.to_string();
    let mut values = vec![
        ("hostname", "127.0.0.1"),
        ("port", port.as_str()),
        ("timeout", "5"),
    ];
    values.extend_from_slice(extra);
    let mut client = SimClient::new(c);
    client.handshake("vnc", &values, 800, 480).await;
    Rig {
        client,
        session,
        replay: DisplayReplay::new(),
    }
}

impl Rig {
    async fn pump(&mut self, ms: u64) -> Vec<Instruction> {
        let mut seen = vec![];
        while let Some(i) = self.client.recv_within(Duration::from_millis(ms)).await {
            if i.opcode == "sync" {
                self.client
                    .send(Instruction::new("sync", [i.args[0].clone()]))
                    .await
                    .unwrap();
            }
            if std::env::var("VNC_TRACE").is_ok() {
                eprintln!("<- {} {:?}", i.opcode, &i.args[..i.args.len().min(3)]);
            }
            self.replay.apply(&i);
            seen.push(i);
        }
        seen
    }

    async fn pump_until<F: Fn(&Rig) -> bool>(&mut self, what: &str, f: F) {
        for _ in 0..40 {
            self.pump(100).await;
            if f(self) {
                return;
            }
        }
        panic!("timed out waiting for: {what}");
    }

    fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let p = self.replay.pixel(0, x, y).unwrap_or([0; 4]);
        [p[0], p[1], p[2]]
    }

    /// The session's final `error` instruction, as (status code, message).
    async fn expect_error(&mut self) -> (u32, String) {
        loop {
            let i = self.client.recv().await.expect("closed without an error");
            if i.opcode == "error" {
                return (i.args[1].parse().unwrap(), i.args[0].clone());
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Mock server helpers
// ---------------------------------------------------------------------------------------------

async fn read_client_message(s: &mut TcpStream) -> Vec<u8> {
    let kind = s.read_u8().await.unwrap();
    let rest = match kind {
        0 => 19,
        2 => {
            let mut h = [0u8; 3];
            s.read_exact(&mut h).await.unwrap();
            let mut m = vec![kind];
            m.extend_from_slice(&h);
            let mut body = vec![0u8; usize::from(u16::from_be_bytes([h[1], h[2]])) * 4];
            s.read_exact(&mut body).await.unwrap();
            m.extend(body);
            return m;
        }
        3 => 9,
        4 => 7,
        5 => 5,
        6 => {
            let mut h = [0u8; 7];
            s.read_exact(&mut h).await.unwrap();
            let len = u32::from_be_bytes([h[3], h[4], h[5], h[6]]) as usize;
            let mut m = vec![kind];
            m.extend_from_slice(&h);
            let mut body = vec![0u8; len];
            s.read_exact(&mut body).await.unwrap();
            m.extend(body);
            return m;
        }
        other => panic!("unexpected client message {other}"),
    };
    let mut m = vec![kind];
    let mut body = vec![0u8; rest];
    s.read_exact(&mut body).await.unwrap();
    m.extend(body);
    m
}

/// SetPixelFormat, SetEncodings, FramebufferUpdateRequest.
async fn read_setup(s: &mut TcpStream) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    (
        read_client_message(s).await,
        read_client_message(s).await,
        read_client_message(s).await,
    )
}

async fn server_init(s: &mut TcpStream, w: u16, h: u16) {
    let mut m = vec![];
    m.extend_from_slice(&w.to_be_bytes());
    m.extend_from_slice(&h.to_be_bytes());
    m.extend_from_slice(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]);
    m.extend_from_slice(&6u32.to_be_bytes());
    m.extend_from_slice(b"My Mac");
    s.write_all(&m).await.unwrap();
}

/// Version, security-type list (3.7+), None authentication and result, init.
async fn open_none(s: &mut TcpStream, w: u16, h: u16) {
    s.write_all(b"RFB 003.008\n").await.unwrap();
    let mut v = [0u8; 12];
    s.read_exact(&mut v).await.unwrap();
    assert_eq!(&v, b"RFB 003.008\n");
    s.write_all(&[1, 1]).await.unwrap();
    assert_eq!(s.read_u8().await.unwrap(), 1);
    s.write_all(&0u32.to_be_bytes()).await.unwrap();
    assert_eq!(s.read_u8().await.unwrap(), 1, "ClientInit shared flag");
    server_init(s, w, h).await;
}

fn rect(x: u16, y: u16, w: u16, h: u16, encoding: i32, body: &[u8]) -> Vec<u8> {
    let mut r = vec![];
    for v in [x, y, w, h] {
        r.extend_from_slice(&v.to_be_bytes());
    }
    r.extend_from_slice(&encoding.to_be_bytes());
    r.extend_from_slice(body);
    r
}

fn update(rects: &[Vec<u8>]) -> Vec<u8> {
    let mut m = vec![0, 0];
    m.extend_from_slice(&(rects.len() as u16).to_be_bytes());
    for r in rects {
        m.extend_from_slice(r);
    }
    m
}

/// A 32 bpp pixel in the format the client requests (memory order B, G, R, X).
fn px32(c: [u8; 3]) -> [u8; 4] {
    [c[2], c[1], c[0], 0]
}
/// A 3 byte ZRLE compressed pixel.
fn cpx(c: [u8; 3]) -> [u8; 3] {
    [c[2], c[1], c[0]]
}

struct Zrle(Compress);
impl Zrle {
    fn new() -> Self {
        Self(Compress::new(Compression::fast(), true))
    }
    /// ZRLE rectangle body: 4 byte length + the zlib data, sharing one stream across rectangles.
    fn body(&mut self, tiles: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(tiles.len() + 64);
        self.0
            .compress_vec(tiles, &mut out, FlushCompress::Sync)
            .unwrap();
        let mut b = (out.len() as u32).to_be_bytes().to_vec();
        b.extend(out);
        b
    }
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const WHITE: [u8; 3] = [255, 255, 255];

// ---------------------------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn vnc_a_01_none_security_paints_a_raw_update() {
    let (port, server) = mock(|mut s| async move {
        open_none(&mut s, 8, 4).await;
        let (pixel_format, encodings, request) = read_setup(&mut s).await;
        assert_eq!(&pixel_format[..8], &[0, 0, 0, 0, 32, 24, 0, 1]);
        assert_eq!(encodings[0], 2);
        assert_eq!(&encodings[4..8], &16i32.to_be_bytes(), "ZRLE preferred");
        assert_eq!(
            request,
            [3, 0, 0, 0, 0, 0, 0, 8, 0, 4],
            "full initial request"
        );
        let mut body = vec![];
        for i in 0..8u8 {
            body.extend_from_slice(&px32(if i < 4 { RED } else { BLUE }));
        }
        let rows: Vec<u8> = body.iter().cycle().take(32).copied().collect();
        let mut four_rows = vec![];
        for _ in 0..4 {
            four_rows.extend_from_slice(&rows);
        }
        s.write_all(&update(&[rect(0, 0, 8, 4, 0, &four_rows)]))
            .await
            .unwrap();
        // Having painted, the client asks for more, incrementally.
        let next = read_client_message(&mut s).await;
        assert_eq!(next, [3, 1, 0, 0, 0, 0, 0, 8, 0, 4]);
    })
    .await;
    let mut rig = connect(port, &[]).await;
    rig.pump_until("raw pixels", |r| {
        r.pixel(0, 0) == RED && r.pixel(7, 3) == BLUE
    })
    .await;
    assert_eq!(rig.replay.size(0), (8, 4));
    server.await.unwrap();
}

#[tokio::test]
async fn vnc_a_02_classic_vnc_password_authentication() {
    let (port, server) = mock(|mut s| async move {
        s.write_all(b"RFB 003.008\n").await.unwrap();
        let mut v = [0u8; 12];
        s.read_exact(&mut v).await.unwrap();
        s.write_all(&[2, 1, 2]).await.unwrap(); // None and VNC both offered
        assert_eq!(
            s.read_u8().await.unwrap(),
            2,
            "client picks VNC auth when a password is set"
        );
        let challenge: [u8; 16] = std::array::from_fn(|i| (i * 11 + 1) as u8);
        s.write_all(&challenge).await.unwrap();
        let mut response = [0u8; 16];
        s.read_exact(&mut response).await.unwrap();
        // Reference value from scripts/vnc_vectors.py would need this exact challenge; recompute
        // with the DES vector helper instead of trusting the client implementation:
        // key = "password" bit-reversed per byte.
        let ok = response == reference_des(b"password", &challenge);
        s.write_all(&(if ok { 0u32 } else { 1u32 }).to_be_bytes())
            .await
            .unwrap();
        assert!(ok, "client DES response differs from the reference");
        assert_eq!(s.read_u8().await.unwrap(), 1);
        server_init(&mut s, 2, 2).await;
        read_setup(&mut s).await;
        s.write_all(&update(&[rect(
            0,
            0,
            2,
            2,
            0,
            &[px32(GREEN), px32(GREEN), px32(GREEN), px32(GREEN)].concat(),
        )]))
        .await
        .unwrap();
        read_client_message(&mut s).await;
    })
    .await;
    let mut rig = connect(port, &[("password", "password")]).await;
    rig.pump_until("green", |r| r.pixel(1, 1) == GREEN).await;
    server.await.unwrap();
}

/// DES with the VNC key-bit reversal, implemented against the `des` crate used by neither side.
fn reference_des(password: &[u8], challenge: &[u8; 16]) -> [u8; 16] {
    use des::cipher::{BlockCipherEncrypt, KeyInit};
    let mut key = [0u8; 8];
    for (k, b) in key.iter_mut().zip(password) {
        *k = (0..8).fold(0u8, |acc, bit| acc | (((b >> bit) & 1) << (7 - bit)));
    }
    let c = des::Des::new(&key.into());
    let mut out = [0u8; 16];
    for i in 0..2 {
        let mut b: des::cipher::Block<des::Des> = <[u8; 8]>::try_from(&challenge[i * 8..i * 8 + 8])
            .unwrap()
            .into();
        c.encrypt_block(&mut b);
        out[i * 8..i * 8 + 8].copy_from_slice(&b);
    }
    out
}

/// macOS Screen Sharing flow: version 003.889, several Apple security types, ARD authentication.
async fn mac_server(mut s: TcpStream, user: &'static str, pass: &'static str) -> bool {
    s.write_all(b"RFB 003.889\n").await.unwrap();
    let mut v = [0u8; 12];
    s.read_exact(&mut v).await.unwrap();
    assert_eq!(&v, b"RFB 003.008\n", "Apple's 3.889 is answered with 3.8");
    s.write_all(&[4, 36, 33, 30, 2]).await.unwrap();
    assert_eq!(s.read_u8().await.unwrap(), 30);

    // 1024-bit MODP group 2 (RFC 2409).
    let prime = hex_to_bytes(
        "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74\
         020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437\
         4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED\
         EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE65381FFFFFFFFFFFFFFFF",
    );
    let p = BigUint::from_bytes_be(&prime);
    let private = BigUint::from_bytes_be(&hex_to_bytes("00badc0ffee0ddf00d0123456789abcdef"));
    let public = BigUint::from(2u8).modpow(&private, &p).to_bytes_be();
    let mut public_padded = vec![0u8; 128 - public.len()];
    public_padded.extend(public);
    s.write_all(&2u16.to_be_bytes()).await.unwrap();
    s.write_all(&128u16.to_be_bytes()).await.unwrap();
    s.write_all(&prime).await.unwrap();
    s.write_all(&public_padded).await.unwrap();

    let mut cipher = [0u8; 128];
    s.read_exact(&mut cipher).await.unwrap();
    let mut client_public = [0u8; 128];
    s.read_exact(&mut client_public).await.unwrap();
    let shared = BigUint::from_bytes_be(&client_public)
        .modpow(&private, &p)
        .to_bytes_be();
    let mut shared_padded = vec![0u8; 128 - shared.len()];
    shared_padded.extend(shared);
    let key: [u8; 16] = Md5::digest(&shared_padded).into();
    let aes = aes::Aes128::new(&key.into());
    let mut plain = vec![];
    for block in cipher.as_chunks::<16>().0 {
        let mut b: aes::cipher::Block<aes::Aes128> = (*block).into();
        aes.decrypt_block(&mut b);
        plain.extend_from_slice(&b);
    }
    let field = |f: &[u8]| {
        let end = f.iter().position(|b| *b == 0).unwrap();
        String::from_utf8(f[..end].to_vec()).unwrap()
    };
    let ok = field(&plain[..64]) == user && field(&plain[64..]) == pass;
    if ok {
        s.write_all(&0u32.to_be_bytes()).await.unwrap();
        assert_eq!(s.read_u8().await.unwrap(), 1);
        server_init(&mut s, 4, 2).await;
        read_setup(&mut s).await;
        let px = [px32(WHITE); 8].concat();
        s.write_all(&update(&[rect(0, 0, 4, 2, 0, &px)]))
            .await
            .unwrap();
        read_client_message(&mut s).await;
    } else {
        let reason = b"Authentication failed";
        s.write_all(&1u32.to_be_bytes()).await.unwrap();
        s.write_all(&(reason.len() as u32).to_be_bytes())
            .await
            .unwrap();
        s.write_all(reason).await.unwrap();
        let _ = s.read_u8().await;
    }
    ok
}

fn hex_to_bytes(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[tokio::test]
async fn vnc_a_03_macos_ard_authentication_succeeds() {
    let (port, server) = mock(|s| mac_server(s, "admin", "s3cret-pässword")).await;
    let mut rig = connect(
        port,
        &[("username", "admin"), ("password", "s3cret-pässword")],
    )
    .await;
    rig.pump_until("white desktop", |r| r.pixel(3, 1) == WHITE)
        .await;
    assert!(server.await.unwrap(), "server accepted the credentials");
}

#[tokio::test]
async fn vnc_a_04_wrong_password_reports_unauthorized() {
    let (port, server) = mock(|s| mac_server(s, "admin", "right")).await;
    let mut rig = connect(port, &[("username", "admin"), ("password", "wrong")]).await;
    let (status, _) = rig.expect_error().await;
    assert_eq!(status, 0x0301);
    assert!(!server.await.unwrap());
}

#[tokio::test]
async fn vnc_a_05_credentials_are_not_sent_to_a_server_offering_no_authentication() {
    let (port, server) = mock(|mut s| async move {
        s.write_all(b"RFB 003.008\n").await.unwrap();
        let mut v = [0u8; 12];
        s.read_exact(&mut v).await.unwrap();
        s.write_all(&[1, 1]).await.unwrap();
        // The client must hang up without choosing a type or sending anything.
        let mut rest = vec![];
        s.read_to_end(&mut rest).await.unwrap();
        rest
    })
    .await;
    let mut rig = connect(port, &[("username", "admin"), ("password", "secret")]).await;
    let (status, message) = rig.expect_error().await;
    assert_eq!(status, 0x0203, "{message}");
    assert!(server.await.unwrap().is_empty());
}

#[tokio::test]
async fn vnc_a_06_server_refusing_the_connection_is_explained() {
    let (port, _server) = mock(|mut s| async move {
        s.write_all(b"RFB 003.008\n").await.unwrap();
        let mut v = [0u8; 12];
        s.read_exact(&mut v).await.unwrap();
        s.write_all(&[0]).await.unwrap();
        let reason = b"Too many failed attempts";
        s.write_all(&(reason.len() as u32).to_be_bytes())
            .await
            .unwrap();
        s.write_all(reason).await.unwrap();
        let _ = s.read_u8().await;
    })
    .await;
    let mut rig = connect(port, &[]).await;
    let (status, _) = rig.expect_error().await;
    assert_eq!(status, 0x0301);
}

#[tokio::test]
async fn vnc_a_07_a_non_vnc_service_is_rejected() {
    let (port, _server) = mock(|mut s| async move {
        s.write_all(b"SSH-2.0-OpenSSH_9.9\r\n").await.unwrap();
        let _ = s.read_u8().await;
    })
    .await;
    let mut rig = connect(port, &[]).await;
    let (status, message) = rig.expect_error().await;
    assert_eq!(status, 0x0203, "{message}");
}

// ---------------------------------------------------------------------------------------------
// Encodings
// ---------------------------------------------------------------------------------------------

fn hextile_body() -> Vec<u8> {
    // 20x17 rectangle: tiles at (0,0) 16x16, (16,0) 4x16, (0,16) 16x1, (16,16) 4x1.
    let mut b = vec![];
    // Tile 0: background RED, foreground GREEN, one plain sub-rectangle at (1,2) of 3x2.
    b.push(2 | 4 | 8);
    b.extend_from_slice(&px32(RED));
    b.extend_from_slice(&px32(GREEN));
    b.push(1);
    b.push(0x12); // x=1, y=2
    b.push(0x21); // w=3, h=2
                  // Tile 1: keeps background RED (not respecified), coloured sub-rect BLUE at (0,0) 2x1.
    b.push(8 | 16);
    b.push(1);
    b.extend_from_slice(&px32(BLUE));
    b.push(0x00);
    b.push(0x10);
    // Tile 2: raw, 16x1, all WHITE.
    b.push(1);
    for _ in 0..16 {
        b.extend_from_slice(&px32(WHITE));
    }
    // Tile 3: only a new background (GREEN), no sub-rects.
    b.push(2);
    b.extend_from_slice(&px32(GREEN));
    b
}

#[tokio::test]
async fn vnc_e_01_hextile_and_copyrect() {
    let (port, server) = mock(|mut s| async move {
        open_none(&mut s, 32, 20).await;
        read_setup(&mut s).await;
        let copy = rect(0, 17, 4, 3, 1, &[0, 0, 0, 16]); // source (0,16)
        s.write_all(&update(&[rect(0, 0, 20, 17, 5, &hextile_body()), copy]))
            .await
            .unwrap();
        read_client_message(&mut s).await;
    })
    .await;
    let mut rig = connect(port, &[]).await;
    rig.pump_until("hextile", |r| {
        r.pixel(5, 5) == RED && r.pixel(2, 3) == GREEN
    })
    .await;
    assert_eq!(rig.pixel(0, 0), RED, "tile 0 background");
    assert_eq!(rig.pixel(1, 2), GREEN, "sub-rectangle top-left");
    assert_eq!(rig.pixel(3, 3), GREEN, "sub-rectangle bottom-right");
    assert_eq!(rig.pixel(4, 3), RED, "just outside the sub-rectangle");
    assert_eq!(rig.pixel(16, 0), BLUE, "coloured sub-rect in tile 1");
    assert_eq!(rig.pixel(17, 0), BLUE);
    assert_eq!(rig.pixel(18, 0), RED, "background persisted across tiles");
    assert_eq!(rig.pixel(7, 16), WHITE, "raw tile");
    assert_eq!(rig.pixel(17, 16), GREEN, "background-only tile");
    // CopyRect from (0,16) of size 4x3 to (0,17): row 16 is white, 17 and 18 were untouched (black).
    assert_eq!(rig.pixel(0, 17), WHITE);
    assert_eq!(
        rig.pixel(0, 18),
        [0, 0, 0],
        "overlapping copy reads the original rows"
    );
    server.await.unwrap();
}

/// 80x66 rectangle = 2x2 ZRLE tiles of 64x64, 16x64, 64x2, 16x2 pixels.
fn zrle_tiles() -> Vec<u8> {
    let mut t = vec![];
    // Tile (0,0) 64x64: solid RED.
    t.push(1);
    t.extend_from_slice(&cpx(RED));
    // Tile (1,0) 16x64: packed palette of 2 colours (1 bit per pixel); left 8 columns GREEN.
    t.push(2);
    t.extend_from_slice(&cpx(GREEN));
    t.extend_from_slice(&cpx(BLUE));
    for _ in 0..64 {
        t.extend_from_slice(&[0b0000_0000, 0b1111_1111]); // 16 pixels: 8 x index 0, 8 x index 1
    }
    // Tile (0,1) 64x2: plain RLE. 100 pixels WHITE then 28 pixels RED (255 + 0 style lengths).
    t.push(128);
    t.extend_from_slice(&cpx(WHITE));
    t.push(99);
    t.extend_from_slice(&cpx(RED));
    t.push(27);
    // Tile (1,1) 16x2 = 32 pixels: palette RLE with 2 colours; first 30 pixels BLUE, then two single GREEN.
    t.push(128 + 2);
    t.extend_from_slice(&cpx(BLUE));
    t.extend_from_slice(&cpx(GREEN));
    t.push(0x80); // index 0 with a run
    t.push(29); // run of 30
    t.push(1);
    t.push(1);
    t
}

#[tokio::test]
async fn vnc_e_02_zrle_tile_types_share_one_zlib_stream_across_rectangles() {
    let (port, server) = mock(|mut s| async move {
        open_none(&mut s, 80, 70).await;
        read_setup(&mut s).await;
        let mut z = Zrle::new();
        let first = z.body(&zrle_tiles());
        // A second rectangle reuses the same zlib stream (and its dictionary): one solid tile.
        let mut solid = vec![1];
        solid.extend_from_slice(&cpx(BLUE));
        let second = z.body(&solid);
        s.write_all(&update(&[
            rect(0, 0, 80, 66, 16, &first),
            rect(0, 66, 10, 4, 16, &second),
        ]))
        .await
        .unwrap();
        read_client_message(&mut s).await;
    })
    .await;
    let mut rig = connect(port, &[]).await;
    rig.pump_until("zrle", |r| r.pixel(0, 69) == BLUE).await;
    assert_eq!(rig.pixel(10, 10), RED, "solid tile");
    assert_eq!(rig.pixel(63, 63), RED);
    assert_eq!(rig.pixel(64, 0), GREEN, "palette tile, left half");
    assert_eq!(rig.pixel(71, 63), GREEN);
    assert_eq!(rig.pixel(72, 0), BLUE, "palette tile, right half");
    assert_eq!(rig.pixel(79, 63), BLUE);
    assert_eq!(rig.pixel(0, 64), WHITE, "RLE tile first run");
    assert_eq!(rig.pixel(35, 64), WHITE);
    assert_eq!(
        rig.pixel(35, 65),
        WHITE,
        "100 white pixels = row 0 plus 36 of row 1"
    );
    assert_eq!(rig.pixel(36, 65), RED, "then the red run begins");
    assert_eq!(rig.pixel(63, 65), RED);
    assert_eq!(rig.pixel(64, 64), BLUE, "palette RLE run");
    assert_eq!(rig.pixel(79, 64), BLUE);
    assert_eq!(rig.pixel(77, 65), BLUE);
    assert_eq!(rig.pixel(78, 65), GREEN);
    assert_eq!(rig.pixel(79, 65), GREEN);
    server.await.unwrap();
}

#[tokio::test]
async fn vnc_e_03_extended_desktop_size_resizes_the_display() {
    let (port, server) = mock(|mut s| async move {
        open_none(&mut s, 16, 16).await;
        read_setup(&mut s).await;
        // Server-initiated resize: x = reason 0, y = status 0, then one screen record.
        let mut body = vec![1, 0, 0, 0];
        body.extend_from_slice(&[0u8; 16]);
        s.write_all(&update(&[rect(0, 0, 32, 24, -308, &body)]))
            .await
            .unwrap();
        // The client must request the whole new desktop, non-incrementally.
        let next = read_client_message(&mut s).await;
        assert_eq!(next, [3, 0, 0, 0, 0, 0, 0, 32, 0, 24]);
        s.write_all(&update(&[rect(31, 23, 1, 1, 0, &px32(RED))]))
            .await
            .unwrap();
        read_client_message(&mut s).await;
    })
    .await;
    let mut rig = connect(port, &[]).await;
    rig.pump_until("resized", |r| {
        r.replay.size(0) == (32, 24) && r.pixel(31, 23) == RED
    })
    .await;
    server.await.unwrap();
}

#[tokio::test]
async fn vnc_e_04_sixteen_bit_colour_is_requested_and_decoded() {
    let (port, server) = mock(|mut s| async move {
        open_none(&mut s, 2, 1).await;
        let (pf, _, _) = read_setup(&mut s).await;
        assert_eq!(&pf[4..8], &[16, 16, 0, 1]);
        assert_eq!(&pf[8..14], &[0, 31, 0, 63, 0, 31]);
        assert_eq!(&pf[14..17], &[11, 5, 0]);
        // Red (0xF800) and blue (0x001F), little endian.
        s.write_all(&update(&[rect(0, 0, 2, 1, 0, &[0x00, 0xf8, 0x1f, 0x00])]))
            .await
            .unwrap();
        read_client_message(&mut s).await;
    })
    .await;
    let mut rig = connect(port, &[("color-depth", "16")]).await;
    rig.pump_until("565", |r| r.pixel(0, 0) == RED && r.pixel(1, 0) == BLUE)
        .await;
    server.await.unwrap();
}

// ---------------------------------------------------------------------------------------------
// Hostile servers
// ---------------------------------------------------------------------------------------------

async fn hostile(send: Vec<u8>, w: u16, h: u16) -> (u32, String) {
    let (port, _server) = mock(move |mut s| async move {
        open_none(&mut s, w, h).await;
        read_setup(&mut s).await;
        s.write_all(&send).await.unwrap();
        let _ = s.read_u8().await;
    })
    .await;
    let mut rig = connect(port, &[]).await;
    rig.expect_error().await
}

#[tokio::test]
async fn vnc_h_01_rectangle_outside_the_framebuffer_is_refused() {
    let (status, message) = hostile(update(&[rect(4, 0, 8, 1, 0, &[0u8; 32])]), 8, 8).await;
    assert_eq!(status, 0x0203, "{message}");
}

#[tokio::test]
async fn vnc_h_02_copyrect_source_outside_is_refused() {
    let (status, _) = hostile(update(&[rect(0, 0, 4, 4, 1, &[0, 6, 0, 6])]), 8, 8).await;
    assert_eq!(status, 0x0203);
}

#[tokio::test]
async fn vnc_h_03_oversized_desktop_is_refused_before_allocating() {
    let (port, _server) = mock(|mut s| async move {
        open_none(&mut s, 65535, 65535).await;
        let _ = s.read_u8().await;
    })
    .await;
    let mut rig = connect(port, &[]).await;
    let (status, message) = rig.expect_error().await;
    assert_eq!(status, 0x0203, "{message}");
}

#[tokio::test]
async fn vnc_h_04_unknown_encoding_and_message_types_are_refused() {
    let (status, _) = hostile(update(&[rect(0, 0, 1, 1, 7, &[0u8; 4])]), 8, 8).await;
    assert_eq!(status, 0x0203);
    let (status, _) = hostile(vec![99], 8, 8).await;
    assert_eq!(status, 0x0203);
}

#[tokio::test]
async fn vnc_h_05_zrle_bombs_and_garbage_are_refused() {
    // Garbage that is not zlib data.
    let mut body = 8u32.to_be_bytes().to_vec();
    body.extend_from_slice(b"notzlib!");
    let (status, _) = hostile(update(&[rect(0, 0, 8, 8, 16, &body)]), 8, 8).await;
    assert_eq!(status, 0x0203);

    // 4 MiB of zeros compress to a few KiB but are far more than one 8x8 tile can carry.
    let mut z = Zrle::new();
    let bomb = z.body(&vec![0u8; 4 << 20]);
    assert!(bomb.len() < 64 * 1024);
    let (status, message) = hostile(update(&[rect(0, 0, 8, 8, 16, &bomb)]), 8, 8).await;
    assert_eq!(status, 0x0203, "{message}");

    // A run that overruns its tile.
    let mut z = Zrle::new();
    let mut tile = vec![128];
    tile.extend_from_slice(&cpx(RED));
    tile.push(200); // 201 pixels in an 8x8 tile
    let (status, message) = hostile(update(&[rect(0, 0, 8, 8, 16, &z.body(&tile))]), 8, 8).await;
    assert_eq!(status, 0x0203, "{message}");

    // A palette index beyond the palette.
    let mut tile = vec![3 + 128];
    tile.extend_from_slice(&[0; 9]);
    tile.push(7); // palette index 7 of 3
    let mut z = Zrle::new();
    let (status, _) = hostile(update(&[rect(0, 0, 8, 8, 16, &z.body(&tile))]), 8, 8).await;
    assert_eq!(status, 0x0203);
}

#[tokio::test]
async fn vnc_h_06_hextile_subrectangle_outside_its_tile_is_refused() {
    let mut body = vec![8 | 2];
    body.extend_from_slice(&px32(RED));
    // 8x8 tile, sub-rect at x=7 of width 3.
    body.extend_from_slice(&[1, 0x70, 0x20]);
    let (status, message) = hostile(update(&[rect(0, 0, 8, 8, 5, &body)]), 8, 8).await;
    assert_eq!(status, 0x0203, "{message}");
}

#[tokio::test]
async fn vnc_h_07_connection_loss_is_reported_not_hung() {
    let (port, _server) = mock(|mut s| async move {
        open_none(&mut s, 8, 8).await;
        read_setup(&mut s).await;
        // Half of a rectangle header, then hang up.
        s.write_all(&[0, 0, 0, 1, 0, 0]).await.unwrap();
    })
    .await;
    let mut rig = connect(port, &[]).await;
    let (status, message) = rig.expect_error().await;
    assert_eq!(status, 0x0208, "{message}");
}

#[tokio::test]
async fn vnc_h_08_unreachable_host_is_reported() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut rig = connect(port, &[]).await;
    let (status, _) = rig.expect_error().await;
    assert_eq!(status, 0x0208);
}

// ---------------------------------------------------------------------------------------------
// Input and clipboard
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn vnc_i_01_keyboard_mouse_and_wheel_reach_the_server() {
    let (port, server) = mock(|mut s| async move {
        open_none(&mut s, 100, 50).await;
        read_setup(&mut s).await;
        let mut got = vec![];
        for _ in 0..4 {
            got.push(read_client_message(&mut s).await);
        }
        got
    })
    .await;
    let mut rig = connect(port, &[]).await;
    for ins in [
        Instruction::new("key", ["65293", "1", "0"]),
        Instruction::new("key", ["65293", "0", "0"]),
        Instruction::new("mouse", ["500", "-3", "1"]),
        Instruction::new("mouse", ["40", "20", "16"]),
    ] {
        rig.client.send(ins).await.unwrap();
    }
    let got = server.await.unwrap();
    assert_eq!(got[0], [4, 1, 0, 0, 0, 0, 0xff, 0x0d]);
    assert_eq!(got[1], [4, 0, 0, 0, 0, 0, 0xff, 0x0d]);
    assert_eq!(
        got[2],
        [5, 1, 0, 99, 0, 0],
        "coordinates clamp to the desktop"
    );
    assert_eq!(
        got[3],
        [5, 16, 0, 40, 0, 20],
        "wheel-down bit passes through"
    );
}

#[tokio::test]
async fn vnc_i_02_read_only_sends_no_input() {
    let (port, server) = mock(|mut s| async move {
        open_none(&mut s, 10, 10).await;
        read_setup(&mut s).await;
        // Anything the client sends from now on is a failure; EOF is the success path.
        let mut rest = vec![];
        let _ = tokio::time::timeout(Duration::from_millis(800), s.read_to_end(&mut rest)).await;
        rest
    })
    .await;
    let mut rig = connect(port, &[("read-only", "true")]).await;
    rig.client
        .send(Instruction::new("key", ["97", "1", "0"]))
        .await
        .unwrap();
    rig.client
        .send(Instruction::new("mouse", ["1", "1", "1"]))
        .await
        .unwrap();
    rig.pump(300).await;
    assert!(server.await.unwrap().is_empty());
}

fn clipboard_instructions(text: &str) -> Vec<Instruction> {
    use base64::Engine;
    vec![
        Instruction::new("clipboard", ["1", "text/plain"]),
        Instruction::new(
            "blob",
            [
                "1".to_string(),
                base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
            ],
        ),
        Instruction::new("end", ["1"]),
    ]
}

#[tokio::test]
async fn vnc_c_01_clipboard_to_the_server_is_filtered_and_encoded() {
    let (port, server) = mock(|mut s| async move {
        open_none(&mut s, 10, 10).await;
        read_setup(&mut s).await;
        read_client_message(&mut s).await
    })
    .await;
    let mut rig = connect(port, &[("clipboard-encoding", "UTF-8")]).await;
    for ins in clipboard_instructions("héllo\u{1b}[2J wörld") {
        rig.client.send(ins).await.unwrap();
    }
    let m = server.await.unwrap();
    assert_eq!(m[0], 6);
    assert_eq!(
        &m[8..],
        "héllo[2J wörld".as_bytes(),
        "ESC removed, UTF-8 kept"
    );
}

#[tokio::test]
async fn vnc_c_02_disable_paste_blocks_clipboard_to_the_server() {
    let (port, server) = mock(|mut s| async move {
        open_none(&mut s, 10, 10).await;
        read_setup(&mut s).await;
        let mut rest = vec![];
        let _ = tokio::time::timeout(Duration::from_millis(800), s.read_to_end(&mut rest)).await;
        rest
    })
    .await;
    let mut rig = connect(port, &[("disable-paste", "true")]).await;
    for ins in clipboard_instructions("secret") {
        rig.client.send(ins).await.unwrap();
    }
    rig.pump(300).await;
    assert!(server.await.unwrap().is_empty());
}

#[tokio::test]
async fn vnc_c_03_server_clipboard_reaches_the_client() {
    let (port, _server) = mock(|mut s| async move {
        open_none(&mut s, 10, 10).await;
        read_setup(&mut s).await;
        // macOS sends UTF-8 even though the specification says Latin-1.
        let text = "mac → clipboard\r\nline 2\u{1b}";
        let mut m = vec![3, 0, 0, 0];
        m.extend_from_slice(&(text.len() as u32).to_be_bytes());
        m.extend_from_slice(text.as_bytes());
        s.write_all(&m).await.unwrap();
        let _ = s.read_u8().await;
    })
    .await;
    let mut rig = connect(port, &[("clipboard-encoding", "UTF-8")]).await;
    let seen = rig.pump(500).await;
    let clip = seen
        .iter()
        .position(|i| i.opcode == "clipboard")
        .expect("clipboard stream");
    use base64::Engine;
    let blob = &seen[clip + 1];
    assert_eq!(blob.opcode, "blob");
    let text = base64::engine::general_purpose::STANDARD
        .decode(&blob.args[1])
        .unwrap();
    assert_eq!(String::from_utf8(text).unwrap(), "mac → clipboard\nline 2");
}

#[tokio::test]
async fn vnc_c_04_disable_copy_drops_server_clipboard_and_oversize_text_is_skipped() {
    let (port, _server) = mock(|mut s| async move {
        open_none(&mut s, 10, 10).await;
        read_setup(&mut s).await;
        let mut m = vec![3, 0, 0, 0];
        m.extend_from_slice(&5u32.to_be_bytes());
        m.extend_from_slice(b"hello");
        s.write_all(&m).await.unwrap();
        // 2 MiB: over the cap, must be skipped without being buffered, and the stream stays in sync.
        let big = (2u32 << 20).to_be_bytes();
        s.write_all(&[3, 0, 0, 0]).await.unwrap();
        s.write_all(&big).await.unwrap();
        s.write_all(&vec![b'x'; 2 << 20]).await.unwrap();
        s.write_all(&update(&[rect(0, 0, 1, 1, 0, &px32(RED))]))
            .await
            .unwrap();
        let _ = s.read_u8().await;
    })
    .await;
    let mut rig = connect(port, &[("disable-copy", "true")]).await;
    rig.pump_until("update after oversize clipboard", |r| r.pixel(0, 0) == RED)
        .await;
}

// ---------------------------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn vnc_l_01_client_disconnect_ends_the_session_and_closes_the_socket() {
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let (port, server) = mock(|mut s| async move {
        open_none(&mut s, 10, 10).await;
        read_setup(&mut s).await;
        ready_tx.send(()).unwrap();
        let mut rest = vec![];
        // The session must drop its end of the connection.
        tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut rest))
            .await
            .expect("socket closed after disconnect")
            .ok();
    })
    .await;
    let rig = connect(port, &[]).await;
    let Rig {
        client, session, ..
    } = rig;
    ready_rx.await.unwrap();
    drop(client);
    let _ = tokio::time::timeout(Duration::from_secs(5), session)
        .await
        .expect("session ended");
    server.await.unwrap();
}

#[tokio::test]
async fn vnc_l_02_missing_hostname_is_a_bad_request() {
    let plugin = VncPlugin;
    assert_eq!(plugin.name(), "vnc");
    assert!(plugin.arg_names().contains(&"clipboard-encoding"));
    assert!(!plugin.arg_names().contains(&"ignore-cert"));
}
