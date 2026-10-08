// <name> — NEAR smart contract in lisp-rlm TypeScript dialect.
// Docs: `near-compile skill --stdout`. Type defs: types/lisp-rlm.d.ts.
/// <reference path="../types/lisp-rlm.d.ts" />

// near.db.* is the strings-at-rest store: key() → string|null (unwrap
// with ??), put() writes text EXACTLY as given, has()/del()/keys()
// round it out. Money is a hover-doc alias over string — u128 amounts
// are DECIMAL STRINGS at every boundary; garbage traps at runtime and
// rolls the whole transaction back.
type Money = string;

export function init(): string {
  // re-init must die: tests/counter.scn.json pins the trap
  if (near.db.has("count")) {
    near.abort("ERR_ALREADY_INITIALIZED");
  }
  near.db.put("owner", near.predecessorAccountId());
  near.db.put("count", "0");
  return "ok";
}

export function increment(): string {
  const cur = near.db.key("count") ?? "0";
  near.db.put("count", u128Add(cur, "1"));
  near.event("incremented", { count: cur });
  return "ok";
}

export function get_count(): string {
  return near.db.key("count") ?? "0";
}
