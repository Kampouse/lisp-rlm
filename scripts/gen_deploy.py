#!/usr/bin/env python3
"""gen_deploy.py — build deployable contract dirs from corpus + shims.

corpus/*.lisp must stay battery-pristine (interpreter eagerly compiles
all bodies; wasm-only builtins like near/json_get_str break lisp-run).
So deploy mains are GENERATED: corpus contract + shim file, every time.

Usage: python3 scripts/gen_deploy.py [erc20|safe|safe-sandbox|all]
       python3 scripts/gen_deploy.py safe-sandbox --owners a.near,b.near,c.near
Writes deploy/<name>/main.lisp + near.json (near-compile build reads these).
safe-sandbox: safe corpus with O1/O2/O3 replaced by --owners (3 accounts).
"""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTRACTS = {
    "erc20": ("corpus/erc20.lisp", "deploy/shims/erc20-shims.lisp"),
    "safe":  ("corpus/safe.lisp",  "deploy/shims/safe-shims.lisp"),
    "safe-sandbox": ("corpus/safe.lisp", "deploy/shims/safe-shims.lisp"),
}

def gen(name: str, owners: list[str] | None = None) -> None:
    src, shim = CONTRACTS[name]
    contract = (ROOT / src).read_text()
    if owners:
        assert len(owners) == 3, "need exactly 3 owner accounts"
        contract = contract.replace('(define O1 "alice.near")', f'(define O1 "{owners[0]}")')
        contract = contract.replace('(define O2 "bob.near")',   f'(define O2 "{owners[1]}")')
        contract = contract.replace('(define O3 "carol.near")', f'(define O3 "{owners[2]}")')
        for i, o in enumerate(owners, 1):
            assert f'(define O{i} "{o}")' in contract, f"owner {i} substitution failed"
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
    owners = None
    if "--owners" in sys.argv:
        owners = sys.argv[sys.argv.index("--owners") + 1].split(",")
    for name in (CONTRACTS if which == "all" else [which]):
        gen(name, owners)
