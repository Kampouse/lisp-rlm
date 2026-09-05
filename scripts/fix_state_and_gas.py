#!/usr/bin/env python3
"""Two real product fixes found by functional verification:
1. --state <path> flag (script authors assumed it existed; env-only was a trap)
2. --gas-schedule strict validation (bad field = loud error, not silent default)
"""

PATH = "/Users/asil/dev/lisp-rlm/src/bin/near_mock.rs"
R = []

# F1a: --state flag → set NEAR_MOCK_STATE (single source of truth already reads it)
R.append((
"""    let mut cfg = RunCfg::default();
    cfg.staking = args.iter().any(|a| a == "--staking");""",
"""    // --state <path> mirrors the NEAR_MOCK_STATE env var (same single source
    // of truth); flag wins over a pre-set env value.
    if let Some(p) = flag_val("--state") {
        std::env::set_var("NEAR_MOCK_STATE", p.trim());
    }
    let mut cfg = RunCfg::default();
    cfg.staking = args.iter().any(|a| a == "--staking");""",
))

# F1b: help text
R.append((
"""    println!("  NEAR_MOCK_SEED        pin random_seed (string, zero-padded to 64 hex)");""",
"""    println!("  --state <path>        state file (default /tmp/near-mock-state.bin; = NEAR_MOCK_STATE)");
    println!("  NEAR_MOCK_SEED        pin random_seed (string, zero-padded to 64 hex)");""",
))

# F2: strict gas-schedule field validation
R.append((
"""        let d = GasSchedule::default();
        let g = |k: &str, def: u64| -> u64 {
            v.get(k).and_then(|x| x.as_u64()).unwrap_or(def)
        };""",
"""        let d = GasSchedule::default();
        // Strict: an explicitly present field with the wrong type/value is an
        // error, not a silent fall-back to the default (a typo'd schedule must
        // never masquerade as a calibrated one). Missing fields still default.
        let g = |k: &str, def: u64| -> Result<u64, String> {
            match v.get(k) {
                None | Some(serde_json::Value::Null) => Ok(def),
                Some(x) => x.as_u64().map(|n| {
                    if n > 0 {
                        n
                    } else {
                        Err(format!("gas schedule field '{k}' must be > 0"))
                    })
                    .unwrap_or(Err(format!(
                        "gas schedule field '{k}' must be a non-negative integer, got {x}"
                    )))
                }.unwrap_or_else(|| Err(format!(
                    "gas schedule field '{k}' must be a non-negative integer, got {x}"
                ))),
            }
        };""",
))

for old, new in R:
    s = open(PATH).read()
    n = s.count(old)
    assert n == 1, f"anchor count {n} != 1"
    open(PATH, "w").write(s.replace(old, new))

# Rewrite the Ok(GasSchedule{...}) construction to use the fallible g()
s = open(PATH).read()
import re
m = re.search(r"Ok\(GasSchedule \{", s)
assert m, "Ok(GasSchedule{ not found"
head, tail = s[:m.start()], s[m.end():]
end = tail.find("})")
assert end > 0
body = tail[:end]
new_body = body.replace('g("', 'g("').replace(")?", ")?")
# convert each `field: g("k", d.k),` to `field: g("k", d.k)?,`
import re as _re
new_body = _re.sub(r'(g\("[^"]+", d\.[a-z_]+\))', r"\1?", new_body)
s = head + "Ok(GasSchedule {" + new_body + tail[end:]
open(PATH, "w").write(s)
print("--state flag + strict gas schedule validation applied")
