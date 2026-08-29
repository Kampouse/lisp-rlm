#!/usr/bin/env bash
# mock-drive.sh — clean-state nonce semantics probe: consume 7, 8, then
# replay 7 (expect ALREADY_USED), then 0 (expect TOO_LOW → but 0 is IN
# window when base=0… k=0 sets bit0 → slide → ononce=1), then read state.
set -e
cd "$(dirname "$0")/../../.."   # lisp-rlm root
W=projects/nostr-gov-lisp/target/nostr-gov-lisp.wasm
MOCK=./target/release/near-mock

$MOCK $W reset >/dev/null 2>&1
NOW=$(python3 -c "import time; print(int(time.time()*1e9)+10**12)")

python3 - "$NOW" << 'EOF' > /tmp/steps.jsonl
import sys, json
sys.path.insert(0, "projects/nostr-gov-lisp/tests")
from bip340 import sign, sha
NOW = int(sys.argv[1]); EXP = NOW + 3600*10**9
SK = bytes([0xAA]*32); PK = "6a04ab98d9e4774ad806e302dddeb63bea16b5cb5f223ee77478e861bb583eb3"
def sig(a, n):
    m = f"expires {EXP}.000000000: {a} | nonce: {n} | contract: escrow.test.near"
    return sign(SK, sha(m.encode())).hex()
def cw(name, n):
    return {"method": "create_wallet", "args": {"name": name, "signature": sig(f"create_wallet:{name}", n),
           "expires_at": str(EXP), "nonce": str(n)}}
steps = [
  {"method": "init", "args": {"npub": PK}},
  cw("w7", 7),
  cw("w8", 8),
  cw("w7", 7),        # replay → ALREADY_USED (bit7 set, no slide)
  cw("w9", 9),
  cw("w0", 0),        # bit0 → slide → ononce=1
  cw("w2", 2),        # after slide base=1 → k=1 bit1
  {"method": "get_owner_nonce", "args": {}},
]
for s in steps: print(json.dumps(s))
EOF

while IFS= read -r line; do
  M=$(python3 -c "import json,sys; print(json.loads(sys.stdin.read())['method'])" <<< "$line")
  A=$(python3 -c "import json,sys; print(json.dumps(json.loads(sys.stdin.read())['args']))" <<< "$line")
  V=""
  [ "$M" = "get_owner_nonce" ] && V="--view"
  OUT=$($MOCK $W "$M" "$A" $V 2>&1)
  LOG=$(echo "$OUT" | grep -oE "LOG: [A-Z_]+" | head -1)
  RET=$(echo "$OUT" | grep -oE '📄 .*' | head -1)
  echo "$M → ${LOG:-$RET:-?}"
done < /tmp/steps.jsonl
echo "── storage:"
strings /tmp/near-mock-state.bin | grep -E "^(obm_lo|obm_hi|ononce|owner)" -A1 | head -8