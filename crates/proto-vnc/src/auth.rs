//! RFB authentication schemes: classic VNC (DES challenge) and Apple Remote Desktop
//! (Diffie-Hellman key agreement, AES-128-ECB encrypted credentials), as used by macOS Screen Sharing.

use aes::cipher::{BlockCipherEncrypt, KeyInit};
use md5::{Digest, Md5};
use num_bigint::BigUint;

/// Largest Diffie-Hellman modulus accepted from a server (bytes). ARD uses 128 bytes (1024 bit).
pub const MAX_DH_BYTES: usize = 512;
/// Smallest accepted modulus: anything shorter is trivially breakable.
pub const MIN_DH_BYTES: usize = 64;

/// Classic VNC authentication: the password (at most 8 bytes, zero padded, each key byte bit-reversed)
/// is the DES key that encrypts the 16 byte challenge.
pub fn vnc_response(password: &str, challenge: &[u8; 16]) -> [u8; 16] {
    let mut key = [0u8; 8];
    for (slot, byte) in key.iter_mut().zip(password.bytes()) {
        *slot = byte.reverse_bits();
    }
    let cipher = des::Des::new(&key.into());
    let mut out = [0u8; 16];
    for (i, block) in challenge.as_chunks::<8>().0.iter().enumerate() {
        let mut b: des::cipher::Block<des::Des> = (*block).into();
        cipher.encrypt_block(&mut b);
        out[i * 8..i * 8 + 8].copy_from_slice(&b);
    }
    out
}

/// Fill `buf` from the operating system RNG.
pub fn random_bytes(buf: &mut [u8]) -> Result<(), String> {
    getrandom::fill(buf).map_err(|e| format!("random number generator unavailable: {e}"))
}

/// Parameters offered by an ARD server.
pub struct ArdOffer<'a> {
    pub generator: u16,
    pub prime: &'a [u8],
    pub server_public: &'a [u8],
}

/// Left-pad (or reject) a big-endian number to `len` bytes.
fn to_fixed(n: &BigUint, len: usize) -> Option<Vec<u8>> {
    let bytes = n.to_bytes_be();
    if bytes.len() > len {
        return None;
    }
    let mut out = vec![0u8; len - bytes.len()];
    out.extend_from_slice(&bytes);
    Some(out)
}

/// Encode a credential into its fixed 64 byte field: UTF-8, NUL terminated, remainder random filler.
fn credential_field(text: &str, filler: &[u8; 64]) -> [u8; 64] {
    let mut field = *filler;
    let bytes = text.as_bytes();
    // Keep room for the terminating NUL; never cut a multi-byte character in half.
    let mut n = bytes.len().min(63);
    while !text.is_char_boundary(n) {
        n -= 1;
    }
    field[..n].copy_from_slice(&bytes[..n]);
    field[n] = 0;
    field
}

/// Compute the ARD client reply: the 128 byte encrypted credential block and the client public key.
///
/// `private` is the client's secret exponent (random in production; injectable for tests).
pub fn ard_response(
    offer: &ArdOffer<'_>,
    username: &str,
    password: &str,
    private: &[u8],
    filler: &[u8; 128],
) -> Result<(Vec<u8>, Vec<u8>), String> {
    const BAD: &str = "The server sent unusable Diffie-Hellman parameters.";
    let len = offer.prime.len();
    if !(MIN_DH_BYTES..=MAX_DH_BYTES).contains(&len) || offer.server_public.len() != len {
        return Err(BAD.into());
    }
    let prime = BigUint::from_bytes_be(offer.prime);
    let generator = BigUint::from(offer.generator);
    let server_public = BigUint::from_bytes_be(offer.server_public);
    let one = BigUint::from(1u8);
    // An even modulus or a degenerate generator / public value would let the server (or a
    // man in the middle) force a predictable shared secret.
    if !prime.bit(0) || generator <= one || generator >= prime {
        return Err(BAD.into());
    }
    let upper = &prime - &one;
    if server_public <= one || server_public >= upper {
        return Err("The server sent an invalid Diffie-Hellman public key.".into());
    }
    let exponent = BigUint::from_bytes_be(private);
    if exponent <= one {
        return Err("Weak Diffie-Hellman secret.".into());
    }
    let client_public =
        to_fixed(&generator.modpow(&exponent, &prime), len).ok_or("Diffie-Hellman overflow.")?;
    let shared = to_fixed(&server_public.modpow(&exponent, &prime), len)
        .ok_or("Diffie-Hellman overflow.")?;

    let key: [u8; 16] = Md5::digest(&shared).into();
    let mut plain = [0u8; 128];
    let (user_filler, pass_filler) = filler.split_at(64);
    plain[..64].copy_from_slice(&credential_field(
        username,
        user_filler.try_into().expect("64 bytes"),
    ));
    plain[64..].copy_from_slice(&credential_field(
        password,
        pass_filler.try_into().expect("64 bytes"),
    ));
    let cipher = aes::Aes128::new(&key.into());
    let mut out = Vec::with_capacity(128);
    for block in plain.as_chunks::<16>().0 {
        let mut b: aes::cipher::Block<aes::Aes128> = (*block).into();
        cipher.encrypt_block(&mut b);
        out.extend_from_slice(&b);
    }
    Ok((out, client_public))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Vectors come from scripts/vnc_vectors.py (pycryptodome / hashlib, an independent implementation).
    const PRIME: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74\
020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437\
4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED\
EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE65381FFFFFFFFFFFFFFFF";
    const SERVER_PUB: &str = "7918d89680ab92ce7e10792825847ec546310372ad685a1899dc3b12381cb197ea4b7a89984b17a85ac9bb6b1dca6038d61d275557c86f5615b55cde6b07777c8326cef8290146ccb05c67539a3fd6ad0374f8b864283e4a9617482df646a33ce6f5e1453c5c5a012576f43886d47d6f272508da24ca059907b8db9f288b4e3c";
    const CIPHER: &str = "434737eb2e166707f97172aadd8059a9ef2a17e529644195e6592cf01a2b5374f95bda5dbc140812693e16284a3c55f12ff034e734a30c7c9a194b9a51b55932e83573212214f964c4bcbfdf6d588163e4d9527efa78688551d483e1e237d198b4a33187318838857fa6a6be03d78682c3ad8edd1b857bcff2edeaa201bcce85";
    const CLIENT_PUB: &str = "2cbe86135acb562fea367405c69e05a6167c22c9681179bf4523688b72bd15485a88837fb8844d3cef48971c5ec9baf6f62cadf17a042d8b91f4723fd80c2458894d713b177f93f88b098ea783134d24e5c3b4a4e7413b7b12328be7d5b34f5f8ab292bbf53b334e439c4e1634f317770cb67691ebe938894af2d849ddeab2cb";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn vnc_des_known_answers() {
        assert_eq!(
            hex(&vnc_response("password", b"0123456789abcdef")),
            "5645abeb5f1e6475e8feb11beb66ea19"
        );
        let challenge: [u8; 16] = std::array::from_fn(|i| i as u8);
        assert_eq!(
            hex(&vnc_response("abc", &challenge)),
            "9c22b4f2088c3465a1562c4b9d6edb04"
        );
        // Only the first eight password bytes matter.
        assert_eq!(
            vnc_response("password-and-more", b"0123456789abcdef"),
            vnc_response("password", b"0123456789abcdef")
        );
    }

    #[test]
    fn ard_known_answer() {
        let prime = unhex(PRIME);
        let server_pub = unhex(SERVER_PUB);
        let filler: [u8; 128] = std::array::from_fn(|i| ((i * 7 + 3) & 0xff) as u8);
        let private = unhex("FEDCBA09876543210FEDCBA098765432");
        let offer = ArdOffer {
            generator: 2,
            prime: &prime,
            server_public: &server_pub,
        };
        let (cipher, public) =
            ard_response(&offer, "Administrator", "p\u{e4}ss w0rd", &private, &filler).unwrap();
        assert_eq!(hex(&cipher), CIPHER);
        assert_eq!(hex(&public), CLIENT_PUB);
    }

    #[test]
    fn ard_rejects_degenerate_parameters() {
        let prime = unhex(PRIME);
        let filler = [0u8; 128];
        let private = [5u8; 16];
        let mut one = vec![0u8; 128];
        one[127] = 1;
        let mut minus_one = prime.clone();
        *minus_one.last_mut().unwrap() -= 1;
        for bad_public in [
            vec![0u8; 128],
            one,
            minus_one,
            prime.clone(),
            vec![1u8; 127],
        ] {
            let offer = ArdOffer {
                generator: 2,
                prime: &prime,
                server_public: &bad_public,
            };
            assert!(ard_response(&offer, "u", "p", &private, &filler).is_err());
        }
        let good_pub = unhex(SERVER_PUB);
        let even = {
            let mut p = prime.clone();
            *p.last_mut().unwrap() &= !1;
            p
        };
        let short = prime[..32].to_vec();
        for (g, p) in [(0u16, &prime), (1, &prime), (2, &even), (2, &short)] {
            let offer = ArdOffer {
                generator: g,
                prime: p,
                server_public: &good_pub,
            };
            assert!(ard_response(&offer, "u", "p", &private, &filler).is_err());
        }
        let offer = ArdOffer {
            generator: 2,
            prime: &prime,
            server_public: &good_pub,
        };
        assert!(ard_response(&offer, "u", "p", &[0, 1], &filler).is_err());
    }

    #[test]
    fn long_and_multibyte_credentials_are_truncated_on_a_character_boundary() {
        let filler = [0xAAu8; 64];
        let text = "é".repeat(40); // 80 bytes
        let field = credential_field(&text, &filler);
        assert_eq!(field[62], 0, "62 bytes = 31 characters, then NUL");
        assert!(std::str::from_utf8(&field[..62]).is_ok());
        assert_eq!(field[63], 0xAA);
    }
}
