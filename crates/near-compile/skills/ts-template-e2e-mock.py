#!/usr/bin/env python3
"""e2e-mock.py — end-to-end test for <name> contract against near-mock.

Requires:
  - near-mock on PATH (cargo install near-mock)
  - compiled contract: ../target/<name>.wasm (run build.sh first)
"""
import json, os, shutil, subprocess, sys, tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bip340 import sign, event_id, sha, i2b, mul, b2i

HERE = os.path.dirname(os.path.abspath(__file__))
PROJECT = "<name>"
WASM = os.environ.get(
    "NEAR_MOCK_WASM",
    os.path.abspath(os.path.join(HERE, "..", "target", f"{PROJECT}.wasm")))

NM = os.environ.get("NEAR_MOCK", shutil.which("near-mock") or "")
C = f"{PROJECT}.test.near"
NOW_S = 1787000000


class Chain:
    def __init__(self, state):
        self.state = state

    def call(self, method, args, view=False, dep=0):
        env = {**os.environ, "NEAR_MOCK_NOW": str(NOW_S)}
        if dep:
            env["NEAR_MOCK_ATTACH"] = str(dep)
        out = subprocess.run(
            [NM, "cross", self.state, f"{C}={WASM}", C, method,
             json.dumps(args)] + (["--view"] if view else []),
            capture_output=True, text=True, env=env)
        blob = out.stdout + out.stderr
        ok = out.returncode == 0 and "LOG: ERR_" not in blob
        ret = next((l[2:].strip() for l in blob.splitlines() if l.startswith("📄 ")), "")
        err = next((l.split("LOG: ")[1].strip() for l in blob.splitlines()
                    if "LOG: ERR_" in l), "")
        return ok, ret, err


def main():
    if not NM:
        print("✗ near-mock not found (set NEAR_MOCK= or cargo install near-mock)")
        return 1
    if not os.path.exists(WASM):
        print(f"✗ {WASM} missing — run ../build.sh first")
        return 1

    with tempfile.TemporaryDirectory() as td:
        ch = Chain(os.path.join(td, "mock.bin"))

        steps = [
            ("init", ch.call("init", {})),
            ("increment", ch.call("increment", {})),
            ("get_count", ch.call("get_count", {}, view=True)),
            ("increment_2", ch.call("increment", {})),
            ("get_count_2", ch.call("get_count", {}, view=True)),
            ("get_version", ch.call("get_version", {}, view=True)),
        ]

        print(f"\n{'step':<18} result")
        print("─" * 96)
        for name, (ok, ret, err) in steps:
            print(f"{name:<18} {'OK' if ok else 'FAIL':<6} {err or ret[:70]}")

        need_ok = ["init", "increment", "get_count", "increment_2", "get_count_2", "get_version"]
        by_name = {n: ok for n, (ok, _, _) in steps}
        bad = [n for n in need_ok if not by_name[n]]
        verdict = "ALL GREEN" if not bad else f"FAILED: {bad}"
        print("\nVERDICT:", verdict)
        return 0 if verdict == "ALL GREEN" else 1


if __name__ == "__main__":
    sys.exit(main())