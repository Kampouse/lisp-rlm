import json
import subprocess

WASM = "/tmp/groth16-verifier/target/groth16-verifier.wasm"
MOCK = "/Users/j-p/dev/stuff/lisp-rlm/target/release/near-mock"
CIR = "/Users/j-p/dev/stuff/lisp-rlm/zk/circuit"

v = json.load(open(CIR + "/verify_args.json"))

# tamper: flip a nibble of input[0] (claims a different Cx commitment)
bad = dict(v)
bad["inputs"] = [v["inputs"][0][:8] + ("0" if v["inputs"][0][8] != "0" else "1") + v["inputs"][0][9:], v["inputs"][1]]

for name, args in [("valid", v), ("tampered-Cx", bad)]:
    r = subprocess.run([MOCK, WASM, "verify", json.dumps(args), "--prepaid", "200"],
                       capture_output=True, text=True)
    out = r.stdout + r.stderr
    val = next((l.strip() for l in out.splitlines() if l.startswith("📄")), "?")
    gas = next((l.strip() for l in out.splitlines() if "⛽" in l), "?")
    print(f"{name:12} {val}   {gas}")
