// ── Protocol #16: BLS12-381 threshold multisig (t-of-n) ──────────────
//
// On-chain: state machine + aggregation + pairing gate. Client-side:
// keygen/shares, partial signatures, and Lagrange coefficients (field
// math over the BLS scalar field can't run in i64/u128 — and shouldn't:
// contracts aggregate, clients compute coefficients).
//
//   init(id, pks, t)      owner registers n validator pubkeys (G2, 96B
//                          compressed hex each) + threshold t (t: number)
//   submit(id, msg, i, σi) validator i submits partial sig (G1, 48B);
//                          dedup + message-binding enforced
//   execute(id, coeffs)    once ≥t distinct partials: σ = P1Sum(σi),
//                          apk = G2Multiexp(pk_i, c_i), then
//                          pairingCheck gate (1 = valid)
//   verified(id)           view: aggregate sig if executed, else ""
//
// Point encodings: G1 48B / G2 96B compressed hex — the EIP-2537 ABI the
// engine exposes (hosts 59-65). Numeric discipline: counters live in
// u128-string storage (escrow idiom `?? 0n`); loop bounds come from
// pks stored as ONE concatenated blob (n × 192-hex, fixed stride) —
// jsonArr only reads compile-time-literal keys off the args buffer, so
// stored arrays must be sliced, not re-parsed. Threshold/params bigint.
// coeffs = JSON array of [`${idx}`, coeff-hex-32B] pairs — one per
// submitted partial, client-side Lagrange over the signer subset.

const ZERO: bigint = 0n;

export function init(pks: string, t: number): string {
  let who = near.signerAccountId();
  if ((near.storageGet("bls:own") ?? "") != "") {
    near.abort("already initialized");
  }
  if (t <= 0) {
    near.abort("threshold must be positive");
  }
  let arr = near.jsonArr("pks");
  let n = arr.length;
  if (t > n) {
    near.abort("threshold exceeds validator count");
  }
  let blob = "";
  for (let i = 0; i < n; i++) {
    if (arr[i].length != 192) {
      near.abort("pk must be 96-byte compressed G2 (192 hex)");
    }
    blob = blob + arr[i];
  }
  near.storageSet("bls:own", who);
  near.storageSet("bls:pksblob", blob);
  near.storageSet("bls:t", `${t}`);
  return `ok:${n}:${t}`;
}

function guardInit(): void {
  if ((near.storageGet("bls:own") ?? "") == "") {
    near.abort("not initialized");
  }
}

export function submit(id: string, msg: string, i: number, sig: string): string {
  guardInit();
  let blob = near.storageGet("bls:pksblob") ?? "";
  let n = blob.length / 192;
  if (blob.length != n * 192 || n == 0) {
    near.abort("corrupt pks blob");
  }
  if (i < 0 || i >= n) {
    near.abort("validator index out of range");
  }
  if (sig.length != 96) {
    near.abort("partial sig must be 48-byte compressed G1 (96 hex)");
  }
  let bound = near.storageGet("bls:msg:" + id) ?? "";
  if (bound == "") {
    near.storageSet("bls:msg:" + id, msg);
  } else if (bound != msg) {
    near.abort("message mismatch for this id");
  }
  let key = `bls:sig:${id}:${i}`;
  if ((near.storageGet(key) ?? "") != "") {
    near.abort("partial already submitted");
  }
  near.storageSet(key, sig);
  let cnt = (near.storageGet("bls:cnt:" + id) ?? ZERO) + 1n;
  near.storageSet("bls:cnt:" + id, `${cnt}`);
  return `submitted:${cnt}`;
}

export function count(id: string): string {
  return near.storageGet("bls:cnt:" + id) ?? "0";
}

export function execute(id: string, coeffs: string): string {
  guardInit();
  let t = near.storageGet("bls:t") ?? ZERO;
  let cnt = near.storageGet("bls:cnt:" + id) ?? ZERO;
  if (cnt < t) {
    near.abort("not enough partials");
  }
  if ((near.storageGet("bls:done:" + id) ?? "") != "") {
    near.abort("already executed");
  }
  let msg = near.storageGet("bls:msg:" + id) ?? "";
  if (msg == "") {
    near.abort("unknown message id");
  }
  let pksBlob = near.storageGet("bls:pksblob") ?? "";
  let nPks = pksBlob.length / 192;
  // aggregate σ = P1Sum over submitted partials; mxBlob = pk_i || c_i
  // for each SUBMITTED validator i. Fixed-stride coeff blob (66 chars:
  // 2-hex idx + 64-hex coeff); every submitted validator MUST find its
  // Lagrange coefficient, else abort.
  // NOTE: scratch lets hoisted — `let` inside nested loop/if bodies
  // lowers to an orphaned binding (frontend block bug, 2026-09-02).
  let sigBlob = "";
  let mxBlob = "";
  let found = "";
  let pkc = "";
  let s = "";
  let base = 0;
  let idx = 0;
  let nE = coeffs.length / 66;
  if (coeffs.length != nE * 66) {
    near.abort("coeffs blob must be 66-char entries");
  }
  for (let i = 0; i < nPks; i++) {
    s = near.storageGet(`bls:sig:${id}:${i}`) ?? "";
    if (s != "") {
      sigBlob = sigBlob + s;
      found = "";
      for (let e = 0; e < nE; e++) {
        base = e * 66;
        idx = strToNum(strSlice(coeffs, base, base + 2));
        if (idx == i) {
          found = strSlice(coeffs, base + 2, base + 66);
        }
      }
      if (found == "") {
        near.abort(`missing coefficient for validator ${i}`);
      }
      pkc = strSlice(pksBlob, i * 192, i * 192 + 192) + found;
      mxBlob = mxBlob + pkc;
    }
  }
  let sigma = near.bls12381P1Sum(sigBlob);
  let apk = near.bls12381G2Multiexp(mxBlob);

  // gate: pairingCheck over (σ, apk) and the message point the client
  // submits alongside — 1 = product is identity = valid aggregate sig
  let gate = near.bls12381PairingCheck(`${sigma}${apk}${msg}`);
  if (gate != 1) {
    near.abort("pairing check failed");
  }
  near.storageSet("bls:done:" + id, sigma);
  return `executed:${sigma}`;
}

export function verified(id: string): string {
  return near.storageGet("bls:done:" + id) ?? "";
}
