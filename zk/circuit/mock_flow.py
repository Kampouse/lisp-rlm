import json
import subprocess
import os

WASM = "/tmp/groth16-verifier/target/groth16-verifier.wasm"
MOCK = "/Users/j-p/dev/stuff/lisp-rlm/target/release/near-mock"
CIR = "/Users/j-p/dev/stuff/lisp-rlm/zk/circuit"
ST = "/tmp/g16v-mock.bin"

init_args = open(CIR + "/init_args.json").read().strip()
verify_args = open(CIR + "/verify_args.json").read().strip()

try:
    os.remove(ST)
except FileNotFoundError:
    pass

for method, args in [("init", init_args), ("verify", verify_args)]:
    r = subprocess.run([MOCK, WASM, method, args, "--prepaid", "200"],
                       capture_output=True, text=True)
    out = r.stdout + r.stderr
    lines = [l.strip() for l in out.splitlines()
             if any(k in l for k in ("📄", "❌", "⛽", "LOG", "caused"))]
    print(f"== {method}:")
    for l in lines[:4]:
        print("  ", l)
