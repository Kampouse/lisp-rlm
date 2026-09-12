// ── BN254 (alt_bn128) precompile probes — the "zk verify" surface ────
//
// End-to-end fixture for the hex⇄binary bridge fix (2026-09-11): the
// alt_bn128_* hosts speak BINARY on the NEAR wire; the TS surface speaks
// hex strings. Before the fix the raw hex ASCII was passed straight to
// the host — a 384B pairing gate arrived as 768 ASCII bytes, sailed past
// the len%192==0 check and decoded as garbage points (silent wrong
// result or "AltBn128 invalid input" trap). Every zk-Groth16 verifier
// built on pairing_check was broken.
//
// Wire format (nearcore quirk, byte-faithful): each 32-byte field
// element = [lo u128 LE ‖ hi u128 LE] — NOT plain big-endian.
//   g1_sum       buf = n × (1B sign 00/01 ‖ 64B G1 point)
//   g1_multiexp  buf = n × (64B G1 point ‖ 32B scalar)
//   pairing      buf = n × (64B G1 ‖ 128B G2); G2 = x.c0 ‖ x.c1 ‖ y.c0 ‖ y.c1
//   pairing returns 1 when ∏ e(g1_i, g2_i) == identity (verify passes)

export function g1_sum(): number {
  const buf = near.jsonGetStr("buf") ?? "";
  const out = near.altBn128G1Sum(buf);
  near.log(`g1sum:${out}`);
  return 0;
}

export function g1_multiexp(): number {
  const buf = near.jsonGetStr("buf") ?? "";
  const out = near.altBn128G1Multiexp(buf);
  near.log(`g1mx:${out}`);
  return 0;
}

export function pairing_check(): number {
  const buf = near.jsonGetStr("buf") ?? "";
  const ok = near.altBn128PairingCheck(buf);
  near.logNum(ok);
  near.log(`pairing:${ok}`);
  return 0;
}
