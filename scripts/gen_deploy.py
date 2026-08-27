#!/usr/bin/env python3
"""gen_deploy.py — build deployable contract dirs from corpus + shims.

corpus/*.lisp must stay battery-pristine (interpreter eagerly compiles
all bodies; wasm-only builtins like near/json_get_str break lisp-run).
So deploy mains are GENERATED: corpus contract + shim file, every time.

Usage: python3 scripts/gen_deploy.py [erc20|safe|all]
Writes deploy/<name>/main.lisp + near.json (near-compile build reads these).
"""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTRACTS = {
    "erc20": ("corpus/erc20.lisp", "deploy/shims/erc20-shims.lisp"),
    "safe":  ("corpus/safe.lisp",  "deploy/shims/safe-shims.lisp"),
}

def gen(name: str) -> None:
    src, shim = CONTRACTS[name]
    contract = (ROOT / src).read_text()
    shims = (ROOT / shim).read_text()
    out = ROOT / "deploy" / name
    out.mkdir(parents=True, exist_ok=True)
    (out / "main.lisp").write_text(contract.rstrip("\n") + "\n" + shims)
    (out / "near.json").write_text(
        f'{{"name":"{name}","src":"main.lisp","output":"target/{name}.wasm"}}\n'
    )
    n = len((out / "main.lisp").read_text().splitlines())
    print(f"deploy/{name}/main.lisp generated ({n} lines)")

if __name__ == "__main__":
    which = sys.argv[1] if len(sys.argv) > 1 else "all"
    for name in (CONTRACTS if which == "all" else [which]):
        gen(name)
