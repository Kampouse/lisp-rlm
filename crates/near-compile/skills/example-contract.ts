// Example TS-dialect contract — every idiom here is verified-safe.
// Compile: near-compile example-contract.ts out.wasm
// Test:    near-mock out.wasm get_count '{}' --view

const NAME_CHARS = "abcdefghijklmnopqrstuvwxyz0123456789_-";

function getStr(k: string) {
  return near.storageGet(k) ?? "";
}
function die(m: string) {
  near.log(m);
  near.panic(m);
}

export function get_version() {
  return "1";                     // view: RETURN convention (never mix with jsonReturnStr)
}

export function init() {
  if (strLength(getStr("count")) !== 0) {
    die("ERR_ALREADY_INITIALIZED");
  }
  near.storageSet("count", "0");
  return 0;
}

export function increment() {
  const signer = near.predecessorAccountId();
  if (signer !== "alice.test.near") {
    die("ERR_NOT_ALICE");         // access control by predecessor
  }
  if (!near.depositGte(0, 542)) { // ONE u128 (lo,hi) ≈ 0.01 NEAR; boolean result
    die("ERR_STORAGE_DEPOSIT");
  }
  const cur = strToNum(getStr("count"));
  near.storageSet("count", toStr(cur + 1));
  near.log("EVENT_JSON:{\"standard\":\"nep297\",\"version\":\"1.0.0\",\"event\":\"increment\",\"data\":{}}");
  return 0;
}

export function get_count() {
  return getStr("count");
}

// ── structured JSON building (2026-09-30): prefer jsonSet/jsonQuote over
// hand-concatenating "{\"k\":\"v\"...}" strings — that's where escape bugs
// breed. jsonQuote(v) returns the ENCODED value INCLUDING quotes, so it
// drops straight into a value slot; jsonSet merges a key into a JSON
// object string. Nested payloads (a request JSON embedding a JSON string)
// compose cleanly:
export function request_echo(): string {
  const inner = jsonSet("{}", "content", jsonQuote("hello \"quoted\" world"));
  const payload = jsonSet("{}", "input_data", jsonQuote(inner));
  const req = jsonSet(payload, "payer_account_id", jsonQuote(near.predecessorAccountId()));
  // bigint literals split u128 → (lo64,hi64) at compile time:
  if (!near.depositGte(12000000000000000000000n)) {
    near.panic("fee required");
  }
  return near.storageSet("last_req", req) as unknown as string;
}
// Return conventions (verified against the wasm export wrapper):
//   get_* exports: `return v` → {"result": v} wrap (json_return_str)
//   non-get_ exports: `return v` → RAW bytes (change methods too —
//     the wrapper value_returns any non-nil tail; nil tail = empty bytes)
//   near.returnStr(v): explicit raw return from any fn
// blockTimestamp(): STRING (ns exceed JS safe ints) — slice chars for
// seconds/nonce; blockTimestampNum() for arithmetic.
