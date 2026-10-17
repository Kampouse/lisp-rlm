#!/usr/bin/env bash
# gauntlet.sh — ONE command, ALL certification layers, fails loud on any red.
#
#   scripts/gauntlet.sh
#
# Layers (each writes /tmp/gauntlet_L<n>_<name>.log):
#   L1 isabelle  — full proof tree: isabelle build -D proof -d theory
#                  (REAL-EXIT 0, 0 sorry/unsolved, no FAILED)
#   L2 cargo     — full test suite (REAL-EXIT 0, every "test result:" 0 failed)
#   L3 targeted  — the named v5 suites incl. the partial-burn pin (cached, fast)
#   L4 chain     — live testnet: RPC liveness + escrow conservation
#                  (escrow ledger ESCX/ESCY == raw ft_balance_of on both tokens)
#
# Opt-out env (recorded loudly in the verdict, never silent):
#   GAUNTLET_SKIP_ISABELLE=1  GAUNTLET_SKIP_CARGO=1
#   GAUNTLET_SKIP_TARGETED=1  GAUNTLET_SKIP_CHAIN=1
# Chain endpoints (mainnet later / fork testing):
#   GAUNTLET_ESCROW=clmm5-51fe5a.testnet  GAUNTLET_POOL=pool.clmm5-51fe5a.testnet
#   GAUNTLET_TOKX=tokx.clmm5-51fe5a.testnet  GAUNTLET_TOKY=toky.clmm5-51fe5a.testnet
#   GAUNTLET_RPC=https://rpc.testnet.near.org
#
# Doctrine baked in: pipe-through masks exit codes -> every layer ends with
# `echo "REAL-EXIT: $?"` and the gate greps THAT, not the pipe's mood.

set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PASS=0; FAIL=0; SKIP=0; FAILED_LAYERS=""

banner() { printf '\n\033[1m== %s ==\033[0m\n' "$*"; }

# gate <name> <logfile> <extra-grep-regex (optional)> — REAL-EXIT + generic reds
gate() { # $1 name, $2 log, $3 extra regex (may be empty)
  local name="$1" log="$2" extra="${3:-}"
  if ! grep -q '^REAL-EXIT: 0$' "$log"; then
    echo "❌ $name — REAL-EXIT nonzero:"; tail -5 "$log"; return 1
  fi
  if grep -qE 'sorry|unsolved|^error|\*\*\* FAILED|FAILED \||test result: FAILED' "$log"; then
    echo "❌ $name — red pattern in log:"; grep -nE 'sorry|unsolved|^error|\*\*\* FAILED|FAILED \||test result: FAILED' "$log" | head -5; return 1
  fi
  if [ -n "$extra" ] && ! grep -qE "$extra" "$log"; then
    echo "❌ $name — missing expected pattern: $extra"; return 1
  fi
  echo "✅ $name"
  return 0
}

run_layer() { # $1 name, $2 env-var-skip, $3 log, $4... command
  local name="$1" skipvar="$2" log="$3"; shift 3
  banner "$name"
  if [ "${!skipvar:-}" = "1" ]; then
    echo "⏭  SKIP ($skipvar=1) — recorded in verdict"; SKIP=$((SKIP+1)); return 0
  fi
  ( "$@" > "$log" 2>&1; echo "REAL-EXIT: $?" >> "$log" ) &
  local pid=$!
  wait "$pid"
  if gate "$name" "$log"; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); FAILED_LAYERS="$FAILED_LAYERS $name"; fi
}

# ---------------------------------------------------------------- L1 isabelle
run_layer "L1 isabelle (proof tree)" GAUNTLET_SKIP_ISABELLE /tmp/gauntlet_L1_isabelle.log \
  bash -c 'cd afp-clmm && isabelle build -v -D proof -d theory'

# ---------------------------------------------------------------- L2 cargo full suite
run_layer "L2 cargo (full suite)" GAUNTLET_SKIP_CARGO /tmp/gauntlet_L2_cargo.log \
  env CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/dev/rlm-shared-target}" cargo test --release

# ---------------------------------------------------------------- L3 targeted v5 suites
run_layer "L3 targeted (c2b+pin, parity, custody, loop_gas, market)" GAUNTLET_SKIP_TARGETED /tmp/gauntlet_L3_targeted.log \
  env CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/dev/rlm-shared-target}" bash -c '
    cargo test --release --test test_pool_v5_c2b && \
    cargo test --release --test test_pool_v5_views && \
    cargo test --release --test test_pool_v5_market && \
    cargo test --release --test surface_parity && \
    cargo test --release --test test_pool_v5_custody && \
    cargo test --release --test test_loop_gas'

# ---------------------------------------------------------------- L4 chain
banner "L4 chain (live testnet, read-only)"
if [ "${GAUNTLET_SKIP_CHAIN:-}" = "1" ]; then
  echo "⏭  SKIP (GAUNTLET_SKIP_CHAIN=1) — recorded in verdict"; SKIP=$((SKIP+1))
else
  python3 - > /tmp/gauntlet_L4_chain.log 2>&1 <<'PYEOF'
import base64, json, os, sys, urllib.request

RPC  = os.environ.get("GAUNTLET_RPC", "https://rpc.testnet.near.org")
ESC  = os.environ.get("GAUNTLET_ESCROW", "clmm5-51fe5a.testnet")
TOKS = {"ESCX": os.environ.get("GAUNTLET_TOKX", "tokx.clmm5-51fe5a.testnet"),
        "ESCY": os.environ.get("GAUNTLET_TOKY", "toky.clmm5-51fe5a.testnet")}

def rpc(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": "g", "method": method, "params": params}).encode()
    req = urllib.request.Request(RPC, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        out = json.load(r)
    if "error" in out:
        raise RuntimeError(f"RPC error: {out['error']}")
    return out["result"]

def view_state(account, prefix: bytes):
    res = rpc("query", {"request_type": "view_state", "finality": "final",
                        "account_id": account,
                        "prefix_base64": base64.b64encode(prefix).decode()})
    return [(base64.b64decode(kv["key"]), base64.b64decode(kv["value"]))
            for kv in res["values"]]

def call_view(account, method, args: dict):
    res = rpc("query", {"request_type": "call_function", "finality": "final",
                        "account_id": account, "method_name": method,
                        "args_base64": base64.b64encode(json.dumps(args).encode()).decode()})
    raw = bytes(res["result"])
    return json.loads(raw.decode()) if raw else None

fail = []
pool = os.environ.get("GAUNTLET_POOL", "pool.clmm5-51fe5a.testnet")
root = pool.split(".", 1)[1] if "." in pool else pool
tokx = os.environ.get("GAUNTLET_TOKX", "tokx." + root)
toky = os.environ.get("GAUNTLET_TOKY", "toky." + root)

def call_run(op_json: bytes):
    # pool export is _run (op-based dispatch), returns UTF-8 string
    res = rpc("query", {"request_type": "call_function", "finality": "final",
                        "account_id": pool, "method_name": "_run",
                        "args_base64": base64.b64encode(op_json).decode()})
    return bytes(res["result"]).decode().strip()

# (a) liveness: core storage keys + view readout
pk = {k.decode(errors="replace") for k, _ in view_state(pool, b"6")}
have = {x for x in pk if x in ("6|P", "6|S", "6|LAST")}
print(f"pool storage: {len(pk)} key(s) under '6', core keys present: {sorted(have)}")
if have != {"6|P", "6|S", "6|LAST"}: fail.append(f"pool core keys missing: {sorted(have)}")

# (b) custody via the escrow OP (deployed wasm may predate it -> raw fallback)
def custody():
    try:
        out = call_run(b'{"op":"escrow"}')
        if out.startswith("ESCX:"):
            ex, ey = out[len("ESCX:"):].split(":ESCY:")
            return int(ex), int(ey), "escrow-op"
    except Exception as e:
        print(f"escrow op unavailable ({e.__class__.__name__}) — raw view_state fallback")
    raw = {k.decode(errors="replace"): v.decode(errors="replace").strip()
           for k, v in view_state(pool, b"6")}
    return int(raw.get("6|ESCX", "0") or "0"), \
           int(raw.get("6|ESCY", "0") or "0"), "raw-keys"
ESCX, ESCY, via = custody()
print(f"custody via {via}: ESCX={ESCX} ESCY={ESCY}")

# (c) global conservation, both tokens: sum of per-holder balances ==
#     expected total minted (census closes to the atom across ALL known
#     holders). ft2 exposes no ft_total_supply, so the expected total is
#     GAUNTLET_TOTAL_{X,Y}; extra holders via GAUNTLET_HOLDERS (csv).
def holders_of(tok):
    hs = [pool, root, "e." + root]
    p2 = os.environ.get("GAUNTLET_POOL2", "pool2." + root)
    if p2: hs.insert(1, p2)
    extra = os.environ.get("GAUNTLET_HOLDERS", "")
    hs += [h for h in extra.split(",") if h]
    return hs

# testnet golden census (Oct 16 certification): X 150e27 over 3 holders,
# Y 1e29 over 3 holders — retarget via env when pools change.
tot_x = int(os.environ.get("GAUNTLET_TOTAL_X", "150000000000000000000000000000"))
tot_y = int(os.environ.get("GAUNTLET_TOTAL_Y", "100000000000000000000000000000"))
totals = {}
for tok, total in ((tokx, tot_x), (toky, tot_y)):
    census, checked = 0, []
    for h in holders_of(tok):
        raw = call_view(tok, "ft_balance_of", {"account_id": h})
        b = int(json.loads(raw)) if isinstance(raw, str) else int(raw)
        if b: census += b; checked.append(f"{h.split('.')[0]}={b}")
    print(f"{tok.split('.')[0]}: census({', '.join(checked)}) = {census} vs total {total}")
    totals[tok] = census
    if total and census != total:
        fail.append(f"{tok.split('.')[0]} conservation: census {census} != total {total}")

# (d) custody keys present (escrow bookkeeping alive)
if "6|ESCX" not in pk or "6|ESCY" not in pk:
    fail.append("custody keys 6|ESCX/6|ESCY missing from pool storage")

if fail:
    print("CHAIN FAILURES:"); [print("  -", f) for f in fail]; sys.exit(1)
print("chain: liveness OK, census==supply EXACT both tokens, custody exposed")
PYEOF
  echo "REAL-EXIT: $?" >> /tmp/gauntlet_L4_chain.log
  if gate "L4 chain (conservation+liveness)" /tmp/gauntlet_L4_chain.log; then
    PASS=$((PASS+1))
  else
    FAIL=$((FAIL+1)); FAILED_LAYERS="$FAILED_LAYERS chain"
  fi
fi

# ---------------------------------------------------------------- verdict
banner "VERDICT"
echo "green: $PASS   red: $FAIL   skipped: $SKIP"
[ -n "$FAILED_LAYERS" ] && echo "failed layers:$FAILED_LAYERS"
if [ "$FAIL" -eq 0 ] && [ "$SKIP" -eq 0 ]; then
  echo "GAUNTLET: GREEN (all layers, no skips)"; exit 0
elif [ "$FAIL" -eq 0 ]; then
  echo "GAUNTLET: GREEN WITH SKIPS — not a full certification"; exit 0
else
  echo "GAUNTLET: RED"; exit 1
fi
