# Fuzz targets

Run on Linux with nightly Rust and cargo-fuzz:

```sh
cargo install cargo-fuzz --locked
cd fuzz
cargo +nightly fuzz run parse_instruction -- -max_total_time=3600 -max_len=8192 -rss_limit_mb=2048
```

Targets: `parse_instruction`, `roundtrip`, `telnet_decoder`, `vt_input`, `color_scheme`.
The separate workspace keeps libFuzzer out of the deployed daemon. Corpus seeds are
tracked; crash artifacts and build output are ignored. Turn every confirmed crash
into an ordinary regression test before fixing it.

The stable Windows fallback is `cargo test --workspace -- --ignored` from the repo
root. These deterministic stress tests have finite seeds and iteration counts;
they do not satisfy the planned one-hour or 24-hour libFuzzer acceptance gate.
