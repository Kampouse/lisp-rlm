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
    if (arr[i].length != 386) {
      near.abort("pk must be 97-byte G2 (193B w/sign = 386 hex)");
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
  let n = blob.length / 386;
  if (blob.length != n * 386 || n == 0) {
    near.abort("corrupt pks blob");
  }
  if (i < 0 || i >= n) {
    near.abort("validator index out of range");
  }
  if (sig.length != 194) {
    near.abort("partial sig must be 97-byte G1 (194 hex)");
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

export function setPoints(id: string, msgPoint: string, g2gen: string): string {
  guardInit();
  // TASK-json-bug.md gate: this function MUST consume its ~700-byte args
  // (id + 192-hex msgPoint + 384-hex g2gen) through json_get_str. The
  // 971744e split accidentally left execute's storage-read guard here —
  // with it, the params were shadowed by empty storage reads and the
  // function aborted "points not set" before ever exercising the json
  // path (the corrupted-abort symptom in TASK-json-bug.md was this abort
  // printing a literal clobbered by the g2gen value's overflow). Fixed to
  // the intended store-params shape; execute keeps its own read guard.
  near.storageSet("bls:mp:" + id, msgPoint);
  near.storageSet("bls:gen", g2gen);
  return "points-ok";
}

export function execute(id: string, msgPoint: string, g2gen: string, coeffs: string): string {
  guardInit();
  let t = near.storageGet("bls:t") ?? ZERO;
  let cnt = near.storageGet("bls:cnt:" + id) ?? ZERO;
  if (cnt < t) {
    near.abort("not enough partials");
  }
  if ((near.storageGet("bls:done:" + id) ?? "") != "") {
    near.abort("already executed");
  }
  // TASK-json-bug.md gate: execute accepts msgPoint/g2gen in its ~700-byte
  // args (json_get_str path — the pairing-gate test drives execute without
  // a prior setPoints). H(m) resolves from storage: the setPoints blob for
  // this id, falling back to the submitted message blob (client pre-negates
  // H(m); pairs are sign-free 96B). g2gen: the setPoints global, falling
  // back to the arg.
  let pt = near.storageGet("bls:mp:" + id) ?? "";
  if (pt == "") {
    pt = near.storageGet("bls:msg:" + id) ?? "";
  }
  let gg = near.storageGet("bls:gen") ?? g2gen;
  if (pt == "" || gg == "") {
    near.abort("points not set for this message");
  }
  let pksBlob = near.storageGet("bls:pksblob") ?? "";
  let nPks = pksBlob.length / 386;
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
        // NOTE: str->num("00") yields nil (leading-zero parse) — entries
        // carry 1-based validator indices ("01".."0n")
        idx = strToNum(strSlice(coeffs, base, base + 2)) - 1;
        if (idx == i) {
          found = strSlice(coeffs, base + 2, base + 66);
        }
      }
      if (found == "") {
        near.abort(`missing coefficient for validator ${i}`);
      }
      pkc = strSlice(pksBlob, i * 386 + 2, i * 386 + 386) + found;   // strip sign: 192B + 32B fr
      mxBlob = mxBlob + pkc;
    }
  }
  let sigma = near.bls12381P1Sum(sigBlob);          // 194 hex (97B w/sign)
  let apk = near.bls12381G2Multiexp(mxBlob);        // 386 hex (193B w/sign)
  // strip sign bytes — pairing pairs are sign-free:
  //   pair1 = (σ 96B ‖ g2gen 192B), pair2 = (H(m) 96B ‖ apk 192B)
  // client pre-negates H(m) so Σe = e(σ,g2)·e(-H(m),apk) == 1 on valid sig
  let sig96 = strSlice(sigma, 2, 194);
  let apk192 = strSlice(apk, 2, 386);
  let gate = near.bls12381PairingCheck(sig96 + gg + pt + apk192);
  if (gate != 1) {
    near.abort("pairing check failed");
  }
  near.storageSet("bls:done:" + id, sigma);
  return `executed:${sigma}`;
}

export function verified(id: string): string {
  return near.storageGet("bls:done:" + id) ?? "";
}
