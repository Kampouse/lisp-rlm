#!/bin/bash
# Intent suite — attacks the DESIGN, not just the happy path
# Properties under test:
#   I1  self-admission impossible (no member -> no vouch -> no member cycle)
#   I2  non-members can never vouch (sybil)
#   I3  threshold exact: n-1 vouches admit nobody; promoted locks candidate
#   I4  promoted members gain FULL vouch power (decentralization actually works)
#   I5  candidates isolated (vouches don't bleed across candidates)
#   I6  admin death after renounce is total (no resurrection, no new genesis)
#   I7  deposit gate + atomic rollback on failure
#   I8  double-apply impossible
#   I9  threshold param drives m-of-n (thr=1 = instant web-of-trust)
cd /Users/jean/dev/lisp-rlm
M=target/release/near-mock
W=target/vetting_dao.wasm
Y=1000000000000000000000
PASS=0; FAIL=0
ok(){ PASS=$((PASS+1)); echo "PASS: $1"; }
bad(){ FAIL=$((FAIL+1)); echo "FAIL: $1"; }
intof(){ python3 -c "
d=open('$1','rb').read()
e='📄'.encode(); i=d.rfind(e)
print(int.from_bytes(d[i+5:i+13],'little') if i>=0 else -999)"; }
pan(){ # state signer method json msg
  $M call "$1" dao=$W dao "$3" "$4" --signer "$2" >/tmp/io 2>&1
  if grep -q "PANIC: $5" /tmp/io; then ok "$3 → rejects '$5'"; else bad "$3 expected '$5' got: $(grep PANIC /tmp/io || tail -1 /tmp/io)"; fi
}
act(){ # state signer method json [attach] — pass = no panic
  if [ -n "$5" ]; then $M call "$1" dao=$W dao "$3" "$4" --signer "$2" --attach "$5" >/tmp/io 2>&1
  else $M call "$1" dao=$W dao "$3" "$4" --signer "$2" >/tmp/io 2>&1; fi
  if grep -q "PANIC" /tmp/io; then bad "$3 unexpected: $(grep PANIC /tmp/io)"; else ok "$3 accepted"; fi
}
vw(){ # state method json want label
  $M call "$1" dao=$W dao "$2" "$3" --view >/tmp/vo 2>&1
  G=$(intof /tmp/vo)
  if [ "$G" = "$4" ]; then ok "$5 = $4"; else bad "$5 = $G want $4"; fi
}

S=/tmp/int_main.bin; rm -f $S
$M call $S dao=$W dao init '{"threshold":2,"admin":"gov.testnet"}' --signer gov.testnet >/dev/null 2>&1

echo "--- I7/I9: deposit gate + clean nil"
pan $S mallory.testnet apply '{"candidate":"mallory.testnet"}' "need deposit"
vw $S get_status '{"candidate":"mallory.testnet"}' 0 "status(unknown)"
act $S mallory.testnet apply '{"candidate":"mallory.testnet"}' $Y
vw $S get_status '{"candidate":"mallory.testnet"}' 1 "status(applied)"

echo "--- I1/I2: no self path in, no sybil vouch, admin is not a member"
pan $S mallory.testnet vouch '{"candidate":"mallory.testnet"}' "voter not vetted"
pan $S gov.testnet     vouch '{"candidate":"mallory.testnet"}' "voter not vetted"

echo "--- genesis seed (only admin path in)"
act $S gov.testnet admin_add '{"candidate":"carol.testnet"}'
act $S gov.testnet admin_add '{"candidate":"dave.testnet"}'
act $S gov.testnet admin_add '{"candidate":"gov.testnet"}'
vw $S get_member_count '{}' 3 "members after seed"

echo "--- I3: threshold exactness"
act $S gov.testnet vouch '{"candidate":"mallory.testnet"}'
vw $S get_status '{"candidate":"mallory.testnet"}' 1 "status(1 vouch ≠ promoted)"
act $S dave.testnet vouch '{"candidate":"mallory.testnet"}'
vw $S get_status '{"candidate":"mallory.testnet"}' 2 "status(2 vouches = promoted)"
vw $S get_member_count '{}' 4 "members after promote"
pan $S carol.testnet vouch '{"candidate":"mallory.testnet"}' "not pending"

echo "--- I4: promoted member (mallory) gained vouch power"
act $S mallory.testnet apply '{"candidate":"eve.testnet"}' $Y
act $S mallory.testnet vouch '{"candidate":"eve.testnet"}'
act $S carol.testnet vouch '{"candidate":"eve.testnet"}'
vw $S get_status '{"candidate":"eve.testnet"}' 2 "eve promoted (incl. mallory's vouch)"

echo "--- I5: candidate isolation"
act $S gov.testnet apply '{"candidate":"bob.testnet"}' $Y
act $S gov.testnet vouch '{"candidate":"bob.testnet"}'
vw $S get_status '{"candidate":"bob.testnet"}' 1 "bob still pending (1/2)"

echo "--- I8: double apply"
pan $S eve.testnet apply '{"candidate":"eve.testnet"}' "already known"

echo "--- I6: renounce = total admin death"
act $S gov.testnet renounce_admin '{}'
pan $S gov.testnet admin_add '{"candidate":"frank.testnet"}' "not admin"
pan $S mallory.testnet admin_add '{"candidate":"frank.testnet"}' "not admin"

T=/tmp/int_wot.bin; rm -f $T
echo "--- I9: thr=1 = instant web-of-trust"
$M call $T dao=$W dao init '{"threshold":1,"admin":"gov.testnet"}' --signer gov.testnet >/dev/null 2>&1
act $T gov.testnet admin_add '{"candidate":"zoe.testnet"}'
act $T zoe.testnet apply '{"candidate":"dan.testnet"}' $Y
act $T zoe.testnet vouch '{"candidate":"dan.testnet"}'
vw $T get_status '{"candidate":"dan.testnet"}' 2 "dan promoted by single vouch"

echo ""
echo "INTENT SUITE: $PASS passed, $FAIL failed"