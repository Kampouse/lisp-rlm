// cfg differential generator v2. Oracle-safe: `break`/`continue` only ever
// emitted INSIDE a loop (tracks loop depth). TS twin + plain-JS oracle.
const { FS, PATH, ri, pick, chance } = require("./gen_common.js");

const OUT = PATH.join(__dirname, "corpus", "cfg");
FS.rmSync(OUT, { recursive: true, force: true });
FS.mkdirSync(OUT, { recursive: true });
const FILES = 60;
const FNS_PER_FILE = 8;

function genStmt(d, ctx) {
  if (d <= 0) {
    const t = ri(0, 3);
    if (t === 0) return `acc = acc + ${ri(1, 9)};`;
    if (t === 1) return `i = i + ${ri(1, 3)};`;
    if (t === 2) return `s = s + "${pick(["a", "b", "c", "|", ""])}";`;
    return chance(0.5) ? `if (acc > ${ri(3, 40)}) { return s + "!"; }` : `acc = acc - 1;`;
  }
  const t = ri(0, 8);
  if (t === 0 || t === 1) {
    ctx.loopDepth++;
    const inner = genStmt(d - 1, ctx) + " i = i + 1;";
    const s = `while (i < ${ri(3, 15)}) { ${inner} }`;
    ctx.loopDepth--;
    return s;
  }
  if (t === 2) {
    ctx.loopDepth++;
    const s = `for (let j = 0; j < ${ri(2, 8)}; j++) { ${genStmt(d - 1, ctx)} }`;
    ctx.loopDepth--;
    return s;
  }
  if (t === 3) {
    return `if (acc % ${ri(2, 5)} == 0) { ${genStmt(d - 1, ctx)} } else { ${genStmt(d - 1, ctx)} }`;
  }
  if (t === 4) {
    // continue/break legal only inside a loop
    if (ctx.loopDepth > 0) {
      const kw = chance(0.5) ? "continue" : "break";
      return `if (i % 2 == 0) { ${kw}; } ${genStmt(d - 1, ctx)}`;
    }
    return genStmt(d - 1, ctx);
  }
  if (t === 5) {
    // bare `{...}` blocks are rejected mid-function by the TS frontend —
    // emit a flat unique-named let instead (block-scoping shape via name)
    return `let b${ri(0, 9999)} = acc + ${ri(1, 5)}; acc = acc + 1;`;
  }
  if (t === 6) {
    ctx.loopDepth++;
    const s = `while (true) { ${genStmt(d - 1, ctx)} if (acc > ${ri(5, 25)}) { break; } }`;
    ctx.loopDepth--;
    return s;
  }
  if (t === 7) {
    // u128-ish accumulation through string form (storage-free limb path)
    ctx.loopDepth++;
    const s = `while (i < ${ri(2, 6)}) { acc = acc + ${ri(1, 4)}; i = i + 1; }`;
    ctx.loopDepth--;
    return s;
  }
  return `${genStmt(d - 1, ctx)} ${genStmt(d - 1, ctx)}`;
}

const manifest = [];
for (let f = 0; f < FILES; f++) {
  let src = "";
  let oracleFns = "";
  for (let n = 0; n < FNS_PER_FILE; n++) {
    const name = `f${n}`;
    const ctx = { loopDepth: 0 };
    const body = Array.from({ length: ri(2, 4) }, () => genStmt(ri(1, 3), ctx)).join("\n  ");
    src += `
export function ${name}(n0: number): string {
  let acc = 0;
  let i = 0;
  let s = "";
  ${body}
  return s + "/" + acc;
}
`;
    oracleFns += `
function ${name}(n0) {
  let acc = 0, i = 0, s = "";
  ${body}
  return s + "/" + acc;
}
`;
    manifest.push({ file: `cfg_${String(f).padStart(3, "0")}`, method: name });
  }
  FS.writeFileSync(PATH.join(OUT, `cfg_${String(f).padStart(3, "0")}.ts`), src);
  FS.writeFileSync(
    PATH.join(OUT, `cfg_${String(f).padStart(3, "0")}.oracle.js`),
    oracleFns + `\nmodule.exports = { ${Array.from({ length: FNS_PER_FILE }, (_, i) => `f${i}`).join(", ")} };\n`
  );
}
console.log(JSON.stringify({ files: FILES, fns: FILES * FNS_PER_FILE }));
