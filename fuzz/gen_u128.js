// u128 edge matrix v2.
// Model (matches runtime semantics verified by the repo's own suites):
//   - u128 arith ops TRAP on overflow (Add past MAX, Sub below 0, Div/Mod by 0)
//   - u128 comparisons are BOOL — used directly in if(), never concat'd
//   - entrypoint is atomic: any sub-op overflow traps the whole call
// Each arith case computes r1=op(const,const), r2=op(x,y), r4=op(storage(x),y).
// Each comparison case emits its three results as literal digits via if/else.
const { FS, PATH, U128_LIST } = require("./gen_common.js");

const ARITH = ["Add", "Sub", "Mul", "Div", "Mod"];
const CMP = ["Lt", "Gt", "Eq"];
const ENTRY_PER_FILE = 60;
const OUT = PATH.join(__dirname, "corpus", "u128");
FS.rmSync(OUT, { recursive: true, force: true });
FS.mkdirSync(OUT, { recursive: true });

const manifest = [];
let idx = 0;
function emit(op, pairs) {
  for (let start = 0; start < pairs.length; start += ENTRY_PER_FILE) {
    const chunk = pairs.slice(start, start + ENTRY_PER_FILE);
    const f = `u128_${op.toLowerCase()}_${String(idx).padStart(3, "0")}`;
    const fns = chunk.map(([a, b], i) => {
      if (ARITH.includes(op)) {
        return `
export function e${i}(x: string, y: string): string {
  const A = "${a}";
  const B = "${b}";
  let r1 = u128${op}(A, B);
  let r2 = u128${op}(x, y);
  near.storageSet("k", x);
  let s = near.storageGet("k") ?? "0";
  let r4 = u128${op}(s, y);
  return r1 + "|" + r2 + "|" + r4;
}`;
      }
      return `
export function e${i}(x: string, y: string): string {
  const A = "${a}";
  const B = "${b}";
  let out = "";
  if (u128${op}(A, B)) { out = out + "1"; } else { out = out + "0"; }
  if (u128${op}(x, y)) { out = out + "1"; } else { out = out + "0"; }
  near.storageSet("k", x);
  let s = near.storageGet("k") ?? "0";
  if (u128${op}(s, y)) { out = out + "1"; } else { out = out + "0"; }
  return out;
}`;
    }).join("\n");
    FS.writeFileSync(PATH.join(OUT, f + ".ts"), fns);
    for (let i = 0; i < chunk.length; i++) manifest.push({ file: f, method: `e${i}`, op, a: chunk[i][0], b: chunk[i][1] });
    idx++;
  }
}
for (const op of ARITH) {
  const pairs = [];
  for (const a of U128_LIST) for (const b of U128_LIST) pairs.push([a, b]);
  emit(op, pairs);
}
for (const op of CMP) {
  // smaller matrix for comparisons — they can't trap on value ranges
  const pairs = [];
  const subset = U128_LIST.filter(v => !/^[0x ]/.test(v) && v !== "-1");
  for (const a of subset) for (const b of subset) pairs.push([a, b]);
  emit(op, pairs);
}
FS.writeFileSync(PATH.join(__dirname, "corpus", "u128_manifest.json"), JSON.stringify(manifest));
console.log(JSON.stringify({ files: idx, entries: manifest.length }));
