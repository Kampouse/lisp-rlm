// u128 edge-value matrix — batched: each generated module exposes up to
// ENTRY_PER_FILE entrypoints; each entrypoint applies one op to one (a,b)
// edge pair in 4 forms (const-pair, arg-pair, result-feed, storage round-trip).
const { FS, PATH, U128_LIST } = require("./gen_common.js");

const OPS = ["Add", "Sub", "Mul", "Div", "Mod", "Lt", "Gt", "Eq"];
const ENTRY_PER_FILE = 60;
const OUT = PATH.join(__dirname, "corpus", "u128");
FS.rmSync(OUT, { recursive: true, force: true });
FS.mkdirSync(OUT, { recursive: true });

const manifest = [];
let idx = 0;
for (const op of OPS) {
  const pairs = [];
  for (const a of U128_LIST) for (const b of U128_LIST) pairs.push([a, b]);
  for (let start = 0; start < pairs.length; start += ENTRY_PER_FILE) {
    const chunk = pairs.slice(start, start + ENTRY_PER_FILE);
    const f = `u128_${op.toLowerCase()}_${String(idx).padStart(3, "0")}`;
    const isCmp = op === "Lt" || op === "Gt" || op === "Eq";
    const wrap = isCmp ? (v) => `toStr(${v})` : (v) => v;
    const fns = chunk.map(([a, b], i) => `
export function e${i}(x: string, y: string): string {
  const A = "${a}";
  const B = "${b}";
  let r1 = u128${op}(A, B);
  let r2 = u128${op}(x, y);
  let r3 = u128${op}(r1, r2);
  near.storageSet("k", x);
  let s = near.storageGet("k") ?? "0";
  let r4 = u128${op}(s, r2);
  return ${wrap("r1")} + "|" + ${wrap("r2")} + "|" + ${wrap("r3")} + "|" + ${wrap("r4")};
}`).join("\n");
    FS.writeFileSync(PATH.join(OUT, f + ".ts"), fns);
    for (let i = 0; i < chunk.length; i++) manifest.push({ file: f, method: `e${i}`, op, a: chunk[i][0], b: chunk[i][1] });
    idx++;
  }
}
FS.writeFileSync(PATH.join(__dirname, "corpus", "u128_manifest.json"), JSON.stringify(manifest, null, 1));
console.log(JSON.stringify({ files: idx, entries: manifest.length }));
