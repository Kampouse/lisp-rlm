// ============================================================
// NEP-CAPS — the language surface, as it should look (v1 spec)
// ============================================================
// The ONLY things a user writes differently from today's dialect:
//   1. a caps: manifest line per unit (grants > the baked-in default)
//   2. nothing else — existing code compiles unchanged
// Everything below compiles under: ts_to_lisp_source → parse_all
//   → type_check_program(exprs, true) → compile_near_from_exprs
//   (same pipeline as tests/test_ts_cross.rs)

// ----------------------------------------------------------------
// MANIFEST — the audit trail. Plain metadata, checked, not enforced
// text. Absence = default grants {storage, call}. This IS the
// security review surface: git diff on this block = authority diff.
// ----------------------------------------------------------------
caps: {
  default: ["storage", "call"],          // baked-in; restating is optional
  grant: {
    release:  ["transfer"],              // + baked-ins (cumulative)
    emergencyWithdraw: ["transfer", "signer"],
    adminSet:            ["signer"],
  }
}

// ----------------------------------------------------------------
// CONTRACT
// ----------------------------------------------------------------
const TOKEN = "token.cc.test.near";
const ZERO = 0n;

// ✅ view fn — no grants needed, zero ceremony. storage/call ambient.
//    Verifier pass 4 PROVES this unit's bytecode never calls a
//    transfer-family builtin: the manifest line below it is the whole audit.
export function getTotalDeposits(who: string): string {
  return "deposits:" + (near.storageGet("vd:" + who) ?? ZERO);
}

// ✅ money-touching fn — the manifest grant unlocks transfer builtins
//    for THIS export's body only. The cap is scoped to the export, not
//    the unit: release() could inline transferOut() and lose nothing,
//    but viewTotal() could NOT inline release() (grant doesn't follow).
export async function deposit(buyer: string, amount: bigint): string {
  const res = await near.call(
    TOKEN,
    "ftTransferFrom",
    "{\"from\":\"" + buyer + "\",\"to\":\"" + near.currentAccountId() + "\",\"amount\":" + amount + "}",
    20000000000000,
    0
  );
  if (res == "") { near.abort("token transfer failed"); }
  let cur = near.storageGet("vd:" + buyer) ?? ZERO;
  near.storageSet("vd:" + buyer, cur + amount);
  return "deposited:" + (cur + amount);
}

export function release(buyer: string, seller: string): string {
  // granted: transfer. Every line that moves value is compiler-gated:
  let amt = near.load_u128("esc:" + buyer);
  if (amt == null) { near.abort("no escrow"); }
  near.storageRemove("esc:" + buyer);        // consume BEFORE send: reentrancy order
  near.transfer_u128(seller, amt);           // ← compiles BECAUSE manifest grants it
  near.log("released " + amt + " to " + seller);
  return "released:" + amt;
}

// ✅ signer authority — explicit, scoped, unforgeable.
export function adminSet(key: string, value: string): void {
  // near.signerAccountId() needs Cap<"signer"> — granted above.
  // Body is small and the authority is on the surface:
  if (near.signerAccountId() != ADMIN) { near.abort("not admin"); }
  near.storageSet(key, value);
}

// ----------------------------------------------------------------
// THE SAME FILE, TODAY'S DIALECT — for contrast, the parts the
// compiler CANNOT see (comments are the only audit trail):
// ----------------------------------------------------------------
// export function release(buyer, seller: string): string {
//   let amt = near.load_u128("esc:" + buyer);
//   if (amt == null) { near.abort("no escrow"); }
//   near.storageRemove("esc:" + buyer);
//   near.transfer_u128(seller, amt);   // ← reader must KNOW this is gated;
//                                      //   nothing enforces it. A view fn
//                                      //   could contain this line too.
// }

// ----------------------------------------------------------------
// THE PROMISE PASS — invisible by design
// ----------------------------------------------------------------
// V1 async lowering already enforces one-await (fail-loud, ts_frontend.rs
// lower_async_function). Under caps v1, `res` and every promise handle
// additionally get verifier pass 6 single-use — a lowered continuation
// re-reading a consumed slot is a compile reject with no source change:

export async function escrowThenRelease(buyer: string, seller: string, amount: bigint): string {
  const locked = await near.call(TOKEN, "ftTransferFrom", /* ... */, 20_000_000_000_000n, 0);
  if (locked == "") { near.abort("lock failed"); }
  near.storageSet("esc:" + buyer, "" + amount);
  // pass 6 fact: `locked`'s slot is consumed above; any second read of
  // it in this function's continuation = [verifier] slot read after
  // consume — single-use violation. You never annotate; it just holds.
  return "escrowed:" + amount;
}

// ----------------------------------------------------------------
// WHAT THE TOOL PRINTS (near-compile --caps or near-mock manifest dump)
// ----------------------------------------------------------------
// unit escrow.test.near @ 7512223f:
//   exports:
//     getTotalDeposits    caps: {storage, call}            [default]
//     deposit             caps: {storage, call}            [default]
//     release             caps: {storage, call, transfer}  [granted: transfer]
//     emergencyWithdraw   caps: {storage, call, transfer, signer}
//     adminSet            caps: {storage, call, signer}    [granted: signer]
//   verifier: pass1-5 ✓  pass6 (affine promise slots) ✓
//   authority graph: release ──transfer──▶ any account
//                    adminSet ──signer──▶ identity reads only
// That dump is the security review. One screen.

// ----------------------------------------------------------------
// REJECTS (negative test file lives beside ts_surface2.rs)
// ----------------------------------------------------------------
// T1  export without grant calls transfer      → "near/transfer requires Cap<\"transfer\">"
//     — export `viewTotal` was granted {storage, call}; add to manifest caps.grant
//       or route through an export that has it"
// T2  forged value       const c: Cap<"transfer"> = { __cap: "transfer" }
//     → "type is not assignable — Cap values have no source constructor"
// T3  inline-the-authority: view fn body containing release()'s transfer line
//     → same as T1; grant does not follow inlining — refactor cannot widen power
// T4  promise double-read (lowered continuation reads consumed slot)
//     → "[verifier pass 6] slot N (promise handle @i) read after consume"
// T5  signer read without grant           → "near/signer_account_id requires
//     Cap<\"signer\"> — not granted to export `viewer`"