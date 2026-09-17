// Common generator helpers for the fuzz campaign.
const FS = require("fs");
const PATH = require("path");

const SEED = Number(process.env.FUZZ_SEED || Date.now());
let _s = SEED >>> 0;
function rnd() { // mulberry32
  _s |= 0; _s = (_s + 0x6D2B79F5) | 0;
  let t = Math.imul(_s ^ (_s >>> 15), 1 | _s);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
function ri(a, b) { return a + Math.floor(rnd() * (b - a + 1)); }
function pick(arr) { return arr[ri(0, arr.length - 1)]; }
function chance(p) { return rnd() < p; }

// ── JSON value generation (torture-oriented) ──────────────────────────
const KEYS = ["k", "a", "b", "n", "amt", "x", "y", "K", "Amt", "key with space",
  "key:colon", 'key"quote', "back\\slash", "uni̇́code", "0", "", "x.y", "nested",
  "deep", "o", "arr", "list", "e", "E", "tru", "nul"];
const STRVALS = ["", " ", "abc", "0", "007", "12abc", "-5", "1e3", "0x10",
  '"quoted"', "{brace}", "[bracket]", ":colon", "a,b", "tab\there",
  "line\nbreak", "é", "😀", "\\esc", "\\\\", "null", "true", "trailing "];
const NUMVALS = ["0", "-0", "1", "-1", "007", "1e3", "1E3", "1.5", "-2.25",
  "0.0", "123456789012345678901234567890123456789",
  "340282366920938463463374607431768211455", // u128::MAX
  "340282366920938463463374607431768211456", // u128::MAX+1
  "18446744073709551616", "99999999999999999999999999", "00012", "+5",
  "0.0000000000000000000001"];

function genJson(depth) {
  if (depth <= 0) {
    const t = chance(0.5) ? "string" : "number";
    const v = t === "string" ? pick(STRVALS) : pick(NUMVALS);
    return JSON.stringify(t === "string" ? v : v); // keep numbers as raw text
  }
  const kind = pick(["obj", "obj", "arr", "lit"]);
  if (kind === "arr") {
    const n = ri(1, 4);
    return "[" + Array.from({length: n}, () => genJson(depth - 1)).join(",") + "]";
  }
  if (kind === "lit") return pick(NUMVALS);
  const n = ri(1, 4);
  const pairs = Array.from({length: n}, (_, i) => {
    const key = chance(0.7) ? pick(KEYS) : "f" + i;
    return JSON.stringify(key) + ":" + genJson(depth - 1);
  });
  return "{" + pairs.join(",") + "}";
}

// ── u128 edge value pool ──────────────────────────────────────────────
const U128 = {
  ZERO: "0", ONE: "1", MAX: "340282366920938463463374607431768211455",
  TWO127: "170141183460469231731687303715884105728",
  P1: "340282366920938463463374607431768211456",
  C18: "1000000000000000000",           // 10^18 (chunk size)
  C18m1: "999999999999999999",          // 10^18-1
  C18p1: "1000000000000000001",
  C36: "1000000000000000000000000000000000000",      // 10^36 (2 chunks)
  C36m1: "999999999999999999999999999999999999",
  C54m1: "340282366920938463463374607431768211454",
  LOOSE: "007", PAD: "000", SP: " 5", HEX: "0x10", NEG: "-1",
  BIG2: "21267647932558653966460912964485513216",    // 2^123
};
const U128_LIST = Object.values(U128);

module.exports = { SEED, rnd, ri, pick, chance, genJson, U128, U128_LIST, KEYS, STRVALS, NUMVALS, FS, PATH };
