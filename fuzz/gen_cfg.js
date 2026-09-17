// Control-flow differential generator: random TS-subset programs exercising
// loops, nesting, continue/break/return positions, hoisted decls, u128-ish
// arithmetic, string building. Oracle = plain Node (same semantics).
const { FS, PATH, rnd, ri, pick, chance } = require("./gen_common.js");

const OUT = PATH.join(__dirname, "corpus", "cfg");
FS.rmSync(OUT, { recursive: true, force: true });
FS.mkdirSync(OUT, { recursive: true });
const FILES = 60;
const FNS_PER_FILE = 8;

function genStmt(d, ctx) {
  if (d <= 0) {
    // leaf: mutate accumulator / counter / early return
    const t = ri(0, 3);
    if (t === 0) return `acc = acc + ${ri(1, 9)};`;
    if (t === 1) return `i = i + ${ri(1, 3)};`;
    if (t === 2) return `s = s + "${pick(["a", "b", "c", "|", ""])}";`;
    return chance(0.5) ? `if (acc > ${ri(3, 40)}) { return s + "!"; }` : `acc = acc - 1;`;
  }
  const t = ri(0, 7);
  if (t === 0) {
    return `while (i < ${ri(3, 15)}) { ${genStmt(d - 1, ctx)} i = i + 1; }`;
  }
  if (t === 1) {
    return `for (let j = 0; j < ${ri(2, 8)}; j++) { ${genStmt(d - 1, ctx)} }`;
  }
  if (t === 2) {
    return `if (acc % ${ri(2, 5)} == 0) { ${genStmt(d - 1, ctx)} } else { ${genStmt(d - 1, ctx)} }`;
  }
  if (t === 3 && ctx.allowContinue) {
    return `if (i % 2 == 0) { continue; } ${genStmt(d - 1, ctx)}`;
  }
  if (t === 4 && ctx.allowBreak) {
    return `if (acc > ${ri(10, 30)}) { break; } ${genStmt(d - 1, ctx)}`;
  }
  if (t === 5) {
    // hoisted-style decl inside a block (loop-body decl scoping shape)
    return `{ let t${ri(0, 99)} = acc + ${ri(1, 5)}; acc = acc + 1; }`;
  }
  if (t === 6) {
    return `while (true) { ${genStmt(d - 1, ctx)} if (acc > ${ri(5, 25)}) { break; } }`;
  }
  return `${genStmt(d - 1, ctx)} ${genStmt(d - 1, ctx)}`;
}

const manifest = [];
for (let f = 0; f < FILES; f++) {
  let src = "";
  for (let n = 0; n < FNS_PER_FILE; n++) {
    const name = `f${n}`;
    const body = Array.from({ length: ri(2, 4) }, () => genStmt(ri(1, 3), { allowContinue: true, allowBreak: true })).join("\n  ");
    src += `
export function ${name}(n0: number): string {
  let acc = 0;
  let i = 0;
  let s = "";
  ${body}
  return s + "/" + acc;
}
`;
    // Node oracle: translate TS→JS by stripping types
    const js = `
function ${name}(n0) {
  let acc = 0, i = 0, s = "";
  ${body.replace(/: string/g, "").replace(/: number/g, "")}
  return s + "/" + acc;
}
`;
    manifest.push({ file: `cfg_${String(f).padStart(3, "0")}`, method: name, js });
  }
  FS.writeFileSync(PATH.join(OUT, `cfg_${String(f).padStart(3, "0")}.ts`), src);
  FS.writeFileSync(PATH.join(OUT, `cfg_${String(f).padStart(3, "0")}.oracle.js`), manifest.slice(-FNS_PER_FILE).map(m => m.js).join("\n") + `\nmodule.exports = { ${Array.from({length: FNS_PER_FILE}, (_, i) => `f${i}`).join(", ")} };`);
}
console.log(JSON.stringify({ files: FILES, fns: FILES * FNS_PER_FILE }));
