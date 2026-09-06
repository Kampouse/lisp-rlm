#!/usr/bin/env bash
# Fetch an account's contract state from a NEAR RPC endpoint and emit a
# near-mock state-import dump (RPC-shaped JSON, base64 keys/values).
#
#   fetch_near_state.sh <account> [out.json] [rpc-url]
#
# Defaults: out=<account>.state.json, rpc=https://rpc.mainnet.near.org
# (testnet: https://rpc.testnet.near.org)
#
# Then, offline:
#   near-mock state import <state.bin> <out.json> [--replace-acct]
#   near-mock cross <state.bin> acct=path.wasm acct <method> ...
set -euo pipefail

ACCT="${1:?usage: fetch_near_state.sh <account> [out.json] [rpc-url]}"
OUT="${2:-${ACCT}.state.json}"
RPC="${3:-${NEAR_RPC:-https://rpc.mainnet.near.org}}"

CURL="curl -s --max-time 30"

# 1) contract state (paginated by the node; view_state returns all entries)
RESP=$($CURL "$RPC" -X POST -H 'Content-Type: application/json' -d "{
  \"jsonrpc\": \"2.0\", \"id\": \"dontcare\", \"method\": \"query\",
  \"params\": {\"request_type\": \"view_state\", \"finality\": \"final\", \"account_id\": \"$ACCT\", \"prefix_base64\": \"\"}
}")

# 2) block height for provenance (decorative — the importer ignores it)
BLOCK=$($CURL "$RPC" -X POST -H 'Content-Type: application/json' -d "{
  \"jsonrpc\": \"2.0\", \"id\": \"dontcare\", \"method\": \"block\", \"params\": {\"finality\": \"final\"}
}")

python3 - "$RESP" "$BLOCK" "$ACCT" "$OUT" <<'PY'
import json, sys
resp, block, acct, out = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
r = json.loads(resp)
if "error" in r or "result" not in r:
    sys.exit(f"RPC error: {resp[:400]}")
values = r["result"].get("values", [])
try:
    height = json.loads(block)["result"]["header"]["height"]
except Exception:
    height = None
dump = {"account": acct, "block_height": height,
        "block_hash": r["result"].get("block_hash"), "values": values}
with open(out, "w") as f:
    json.dump(dump, f, indent=1)
print(f"📦 {acct}: {len(values)} keys @ block {height} → {out}")
PY

echo "next:"
echo "  near-mock state import <state.bin> '$OUT'"
echo "  near-mock state dump <state.bin> '$ACCT'"
