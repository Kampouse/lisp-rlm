#!/usr/bin/env python3
"""wasm-input-fuzz — adversarial INPUT fuzzing for compiled lisp-rlm wasm.

Complement to wasm-fuzz.py (which generates hostile CODE with clean
input). This one generates hostile INPUTS for fixed contracts:

  python3 scripts/wasm-input-fuzz.py [--wasm path] [--batch N] [--seed S]

Attack surface: the hand-emitted runtime JSON scanner (json_get_str /
json_get_int read env.input into INPUT_BUF@16384 (16KB), scan for
`"key":` patterns, parse values with escape handling — all raw wasm).

Checks per input:
  OK        contract ran, logs extracted
  TRAP      host error / wasm trap (legit failure mode)
  TIMEOUT   >10s — hang
  PANIC     rust panic in mock — tooling bug
  NOCLOCK   nondeterministic: two runs logged differently

Oracle mode: valid inputs must produce the expected exact log lines
(checked for the built-in probe contract).

Known-realized (pre-fuzz inspection, 2026-08-27):
  - mock read_register OOB silently copies NOTHING (real NEAR traps)
  - INPUT region is 16KB but adjacent memory is RETURN_BUF/heap:
    >16KB inputs corrupt it with no trap, on mainnet too
"""

import argparse
import json
import os
import random
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MOCK = os.path.join(ROOT, "target", "debug", "near-mock")

PROBE = r"""(define (main)
  (println (near/json_get_str "name"))
  (println (near/json_get_int "count"))
  (println (str-length (near/json_get_str "note")))
  (println (if (= (near/json_get_int "count") 7) "seven" "not-seven"))
)
"""

VALID = '{"name":"alice","count":7,"note":"hi"}'

# ---------------------------------------------------------------- generators

def gen_adversarial(seed):
    """Deterministically generate labeled hostile inputs."""
    rng = random.Random(seed)
    cases = []  # (label, args_str_or_bytes)

    def add(label, s):
        cases.append((label, s if isinstance(s, str) else s))

    # -- truncations at every offset of a valid object
    for i in range(1, len(VALID) + 1, max(1, len(VALID) // 24)):
        add(f"trunc@{i}", VALID[:i])

    # -- escapes / control bytes / invalid utf-8 in the value
    esc = ['\\"', "\\\\", "\\u0041", "\\n", "\\/", "a\\", "\\ud83d\\ude00", "\\u00e9"]
    for i, e in enumerate(esc):
        add(f"esc{i}", '{"name":"x' + e + 'y","count":7,"note":"hi"}')
    for i, b in enumerate(["\x00", "\x0a", "\x0d", "\x1f", "\x7f"]):
        add(f"ctrl{i}", '{"name":"a' + b + 'b","count":7,"note":"h"}')
    for i, b in enumerate(["\xff", "\x80", "\xc3", "\xc3\x28"]):
        add(f"badutf{i}", '{"name":"a' + b + 'b","count":7,"note":"h"}')

    # -- value contains the pattern of ANOTHER key (scanner confusion)
    add("pattern-in-value", '{"name":"alice","count":7,"note":"x\\"count\\":99\\"name\\":\\"bob\\"y"}')
    add("pattern-raw-in-value", '{"name":"alice","count":7,"note":"\\"name\\":\\"ev\\""}')
    add("value-looks-like-object", '{"note":"{\\"name\\":\\"z\\",\\"count\\":1}"}')

    # -- duplicate keys (first vs last wins?)
    add("dup-name-first", '{"name":"aa","name":"bb","count":7,"note":"h"}')
    add("dup-count", '{"count":1,"name":"x","count":7,"note":"h"}')

    # -- substring / superstring key confusion
    add("key-nam", '{"nam":"x","count":7,"note":"h"}')
    add("key-nickname", '{"nickname":"x","count":7,"note":"h"}')
    add("key-name-x", '{"namex":"x","count":7,"note":"h"}')
    add("key-notes-before-name", '{"notes":"n","name":"a","count":7,"note":"h"}')
    add("key-count-in-counting", '{"counting":3,"name":"a","count":7,"note":"h"}')

    # -- missing keys
    add("missing-all", "{}")
    add("missing-name", '{"count":7,"note":"h"}')
    add("missing-count", '{"name":"a","note":"h"}')
    add("empty-input", "")

    # -- wrong types for count
    for i, v in enumerate(['"7"', "7.5", "7.0", "-7", "true", "false", "null",
                           "007", "+7", "7e2", "9223372036854775807",
                           "-9223372036854775808", "1152921504606846975",
                           "1152921504606846976", "0", "[7]", "{}"]):
        add(f"count-type{i}", '{"name":"a","count":' + v + ',"note":"h"}')

    # -- wrong types for name
    for i, v in enumerate(["7", "true", "null", "[]", '{}', '""', '["a"]']):
        add(f"name-type{i}", '{"name":' + v + ',"count":7,"note":"h"}')

    # -- whitespace / separators
    add("ws-tight", '{"name":"a","count":7,"note":"h"}'.replace('":', '":'))
    add("ws-space", '{ "name" : "a" , "count" : 7 , "note" : "h" }')
    add("ws-tab-crlf", '{\r\n\t"name"\t:\t"a",\r\n"count":7,\r\n"note":"h"\r\n}')
    add("ws-bom", "﻿" + VALID)
    add("ws-leading-space", "   " + VALID)
    add("ws-trailing", VALID + "   ")
    add("trailing-garbage", VALID + "xx")
    add("leading-garbage", "xx" + VALID)

    # -- non-object top level
    for i, v in enumerate(["[]", '"str"', "42", "true", "null", "[1,2]",
                           '"{\\"name\\":\\"a\\"}"', "{", "}", '{"a"']):
        add(f"top{i}", v)

    # -- malformed structure
    add("unescaped-quote", '{"name":"al"ice","count":7,"note":"h"}')
    add("key-unquoted", "{name:\"a\",\"count\":7,\"note\":\"h\"}")
    add("colon-missing", '{"name" "a","count":7,"note":"h"}')
    add("comma-trailing", '{"name":"a","count":7,"note":"h",}')
    add("nested-object", '{"o":{"name":"inner","count":1},"name":"a","count":7,"note":"h"}')
    add("nested-array", '{"a":[{"name":"inner"}],"name":"a","count":7,"note":"h"}')

    # -- deep nesting (1000 levels)
    add("deep-1000", '{"name":' + "[" * 1000 + "]" * 1000 + ',"count":7,"note":"h"}')

    # -- size attacks: straddle and blow past INPUT_BUF (16KB)
    for label, size in [("16k-exact", 16384), ("16k-over-1", 16385),
                        ("17k", 17 * 1024), ("20k", 20 * 1024),
                        ("24k", 24 * 1024), ("32k", 32 * 1024), ("100k", 100 * 1024)]:
        pad = "x" * size
        add(label, '{"name":"' + pad + '","count":7,"note":"h"}')
    # huge key (pattern > region)
    add("huge-key", '{"' + "k" * 20000 + '":"v","name":"a","count":7,"note":"h"}')
    # long value with the needle pattern near the end
    add("pattern-at-end", '{"name":"' + "y" * 16500 + '","count":7,"note":"h"}')

    # -- many keys (scan cost)
    many = ",".join(f'"k{i}":{i}' for i in range(500))
    add("many-keys", "{" + many + ',"name":"a","count":7,"note":"h"}')

    # -- random byte soup
    for i in range(8):
        n = rng.randint(1, 300)
        soup = bytes(rng.randint(0, 255) for _ in range(n))
        add(f"soup{i}", soup)

    # -- valid-but-nasty unicode
    add("unicode-vals", '{"name":"Ω≈ç√","count":7,"note":"héllo ümlaut"}')
    add("unicode-escape", '{"name":"\\u0041\\u0042","count":7,"note":"h"}')
    add("surrogate-pair", '{"name":"\\ud83d\\ude00","count":7,"note":"h"}')
    add("lone-surrogate", '{"name":"\\ud83d","count":7,"note":"h"}')

    return cases


# ---------------------------------------------------------------- runner

def run_once(wasm, args_bytes, timeout=10):
    """Run near-mock once. Returns (class, logs, stderr_tail)."""
    with tempfile.NamedTemporaryFile(delete=False) as f:
        f.write(args_bytes)
        path = f.name
    try:
        p = subprocess.run(
            [MOCK, wasm, "_run", "@" + path, "--once"],
            capture_output=True, text=True, timeout=timeout,
        )
        logs = [l.strip()[4:].strip() for l in (p.stdout + p.stderr).splitlines()
                if l.strip().startswith("LOG:")]
        tail = (p.stdout + p.stderr)[-400:]
        if "panicked at" in (p.stdout + p.stderr):
            return "PANIC", logs, tail
        if p.returncode != 0 or "Error" in tail or "error" in tail[:200]:
            return "TRAP", logs, tail
        return "OK", logs, tail
    except subprocess.TimeoutExpired:
        return "TIMEOUT", [], ""
    finally:
        os.unlink(path)


def expected_for_probe():
    return ['"alice"', "7", "2", '"seven"']


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--wasm", default=None, help="target .wasm (default: build+use probe)")
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--full", action="store_true", help="run the size attacks too (slow)")
    args = ap.parse_args()

    if args.wasm:
        wasm = args.wasm
        oracle = None
    else:
        # build probe
        probe_lisp = os.path.join(tempfile.mkdtemp(), "probe.lisp")
        wasm = probe_lisp.replace(".lisp", ".wasm")
        with open(probe_lisp, "w") as f:
            f.write(PROBE)
        c = subprocess.run([os.path.join(ROOT, "target", "debug", "near-compile"),
                            probe_lisp, wasm], capture_output=True, text=True)
        if c.returncode != 0:
            print("probe compile failed:", c.stdout + c.stderr)
            sys.exit(1)
        oracle = expected_for_probe()

    cases = gen_adversarial(args.seed)
    if not args.full:
        cases = [(l, a) for (l, a) in cases if not l.startswith(("16k", "17k", "20k", "24k", "32k", "100k", "huge", "pattern-at-end"))]

    # oracle check on the valid input first
    if oracle:
        cls, logs, _ = run_once(wasm, VALID.encode())
        status = "oracle-OK" if logs == oracle else f"ORACLE-MISMATCH cls={cls} logs={logs}"
        print(f"[oracle] {status}")
        if "MISMATCH" in status:
            print("  expected:", oracle)

    counts, flagged = {}, []
    for label, inp in cases:
        data = inp.encode("utf-8", "surrogateescape") if isinstance(inp, str) else inp
        cls1, logs1, tail1 = run_once(wasm, data)
        # determinism: rerun unless it trapped/panicked cleanly
        if cls1 in ("OK",):
            cls2, logs2, _ = run_once(wasm, data)
            if logs1 != logs2:
                cls1 = "NOCLOCK"
        counts[cls1] = counts.get(cls1, 0) + 1
        interesting = cls1 in ("TIMEOUT", "PANIC", "NOCLOCK")
        if oracle and label in ("missing-name", "missing-count", "missing-all", "empty-input"):
            continue
        if interesting:
            flagged.append((label, cls1, logs1, tail1))
        # heuristic divergence reporting: OK runs on obviously-invalid input
        if cls1 == "OK" and label.startswith(("trunc", "unescaped", "key-unquoted", "colon",
                                               "comma", "top", "badutf", "unterm")):
            flagged.append((label, "OK-ON-INVALID", logs1, tail1))

    print(f"\n== wasm-input-fuzz summary ({len(cases)} inputs) ==")
    for k in sorted(counts):
        print(f"  {k:14} {counts[k]}")
    if flagged:
        print("\n-- flagged --")
        for label, cls, logs, tail in flagged[:12]:
            print(f"[{cls}] {label}: logs={logs[:4]} tail={tail[:120]!r}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
