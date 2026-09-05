#!/usr/bin/env python3
"""random_seed: when time is pinned (--now or NEAR_MOCK_BLOCK_HEIGHT), derive
the seed deterministically from (height, ts) instead of wall clock."""

PATH = "/Users/asil/dev/lisp-rlm/src/bin/near_mock.rs"
R = []

R.append((
"""            // Real entropy (time ^ pid, SplitMix64-spread) → 64-char lowercase
            // hex (real NEAR returns raw bytes; the compiler's
            // read_to_register path keeps bytes, but the TS surface stringifies
            // as hex — parity with the ctx battery's `seed.length == 64`
            // probe). Pin with NEAR_MOCK_SEED for reproducible runs.
            let mut z = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x5EED)
                ^ ((std::process::id() as u64) << 32);""",
"""            // Entropy source, in priority order:
            //   1. NEAR_MOCK_SEED pin (explicit reproducibility)
            //   2. --now / NEAR_MOCK_BLOCK_HEIGHT pin → seed = SplitMix64 of
            //      (height, ts): fully deterministic, yet differs per block —
            //      matches real NEAR's per-block random_seed semantics.
            //   3. Wall clock ^ pid (real-ish entropy for non-pinned runs).
            let height = std::env::var("NEAR_MOCK_BLOCK_HEIGHT")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(1000);
            let pinned_ts = mock_cfg().base_ts.map(|t| t as u64);
            let mut z = match (pinned_ts, std::env::var("NEAR_MOCK_SEED").ok()) {
                (Some(ts), None) => {
                    let mut h = height;
                    splitmix64(&mut h) ^ splitmix64(&mut { let mut t = ts; t })
                }
                _ => {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0x5EED)
                        ^ ((std::process::id() as u64) << 32)
                }
            };""",
))

for old, new in R:
    n = PATH and open(PATH).read().count(old)
    assert n == 1, f"anchor count {n} != 1"
    s = open(PATH).read()
    open(PATH, "w").write(s.replace(old, new))
print("random_seed pinned-time determinism applied")
