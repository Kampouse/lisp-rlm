#!/bin/bash
# Q-learning training driver: near-mock cross, state persists in .bin
cd /Users/asil/dev/lisp-rlm
NM=./target/release/near-mock
ACC=qlearn.t.near
MAN="$ACC=/tmp/qlearn.wasm"
ST=/tmp/ql_state.bin
rm -f $ST
q() { $NM cross $ST "$MAN" $ACC qget "{\"k\":\"$1\"}" 2>/dev/null | grep '📄' | head -1; }
echo "initQ: $($NM cross $ST "$MAN" $ACC initQ '{}' 2>/dev/null | grep '📄' | head -1)"
echo "Q[0|L] before training: $(q '0|L')"
s=0
for step in $(seq 1 45); do
  eps=$(( 50 - step * 40 / 45 ))
  out=$($NM cross $ST "$MAN" $ACC act "{\"s\":$s,\"eps\":$eps}" 2>/dev/null | grep '📄' | head -1)
  s2=$(echo "$out" | sed -E 's/.*:(-?[0-9]+):(-?[0-9]+)".*/\2/')
  if [ -z "$s2" ]; then echo "step $step PARSE FAIL: $out"; break; fi
  s=$s2
  if [ $((step % 9)) -eq 0 ]; then echo "step $step (eps=$eps) Q[0|L]=$(q '0|L')  last=$out"; fi
done
echo "Q[0|L] after training:  $(q '0|L')"
echo "Q[0|C]: $(q '0|C')   Q[0|R]: $(q '0|R')"
echo "--- greedy policy run (eps=0), from s=0, 3 acts ---"
s=0
for i in 1 2 3; do
  out=$($NM cross $ST "$MAN" $ACC act "{\"s\":$s,\"eps\":0}" 2>/dev/null | grep '📄' | head -1)
  echo "policy act $i: $out"
  s=$(echo "$out" | sed -E 's/.*:([0-9]+)".*/\1/')
done
echo "--- final table ---"
$NM cross $ST "$MAN" $ACC qtable '{}' 2>/dev/null | grep '📄' | head -1
