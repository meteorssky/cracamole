#!/usr/bin/env python3
"""Independent known-answer vectors for crates/proto-vnc/src/auth.rs (needs pycryptodome)."""
import hashlib
from Crypto.Cipher import AES, DES


def vnc_response(password: bytes, challenge: bytes) -> bytes:
    key = bytes(int(f"{b:08b}"[::-1], 2) for b in (password[:8] + b"\0" * 8)[:8])
    return DES.new(key, DES.MODE_ECB).encrypt(challenge)


def ard_response(prime, g, server_pub, priv, user, pw, filler):
    n = len(prime)
    p = int.from_bytes(prime, "big")
    shared = pow(int.from_bytes(server_pub, "big"), priv, p).to_bytes(n, "big")
    client_pub = pow(g, priv, p).to_bytes(n, "big")
    key = hashlib.md5(shared).digest()

    def field(text: bytes, fill: bytes) -> bytes:
        return text[:63] + b"\0" + fill[len(text[:63]) + 1:]

    plain = field(user, filler[:64]) + field(pw, filler[64:])
    return AES.new(key, AES.MODE_ECB).encrypt(plain), client_pub


if __name__ == "__main__":
    print("vnc", vnc_response(b"password", b"0123456789abcdef").hex())
    print("vnc-short", vnc_response(b"abc", bytes(range(16))).hex())
    # 1024-bit MODP group 2 (RFC 2409) prime.
    prime = bytes.fromhex(
        "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74"
        "020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437"
        "4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED"
        "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE65381FFFFFFFFFFFFFFFF"
    )
    server_priv = 0x1234567890ABCDEF1234567890ABCDEF
    server_pub = pow(2, server_priv, int.from_bytes(prime, "big")).to_bytes(128, "big")
    filler = bytes((i * 7 + 3) & 0xFF for i in range(128))
    priv = 0xFEDCBA09876543210FEDCBA098765432
    ct, cpub = ard_response(prime, 2, server_pub, priv, "Administrator".encode(), "päss w0rd".encode(), filler)
    print("ard-server-pub", server_pub.hex())
    print("ard-cipher", ct.hex())
    print("ard-client-pub", cpub.hex())
