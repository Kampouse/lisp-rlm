// <name> — NEAR smart contract in lisp-rlm TypeScript dialect.
// Docs: near-compile skill (run `near-compile skill --stdout` for full reference).
/// <reference path="../types/lisp-rlm.d.ts" />

const VERSION = "1";

function getStr(k: string): string { return near.storageGet(k) ?? ""; }
function die(m: string): never { near.log(m); near.abort(m); unreachable(); }
function unreachable(): never { near.abort("unreachable"); unreachable(); }

export function init(): number {
  if (strLength(getStr("count")) !== 0) { die("ERR_ALREADY_INITIALIZED"); }
  near.storageSet("count", "0");
  return 0;
}

export function increment(): number {
  const cur = strToNum(getStr("count"));
  near.storageSet("count", toStr(cur + 1));
  return 0;
}

export function get_count(): string { return getStr("count"); }
export function get_version(): string { return VERSION; }