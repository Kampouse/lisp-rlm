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
