// JSON torture v3. Model:
//  - tx input = canonical flat object { probeKey: "7", ... } (plain, tight
//    colons, no escapes) — direct lookups (dynamic + literal) hit or miss on
//    it with value "7" (string→"7", int→7).
//  - stored doc (storageSet inside entrypoint) = tortured JSON — reached via
//    the 2-arg form jsonGetStr(key, doc) / jsonGetInt(key, doc) + dot-paths.
// Oracle semantics (documented):
//  - strings  → unescaped content; objects/arrays → raw balanced span
//  - ints     → prefix-digit parse of raw span, non-num → null
//  - misses   → null ("~" / -1 in the result string)
const { FS, PATH, genJson, KEYS, ri, pick, chance } = require("./gen_common.js");

const OUT = PATH.join(__dirname, "corpus", "json");
FS.rmSync(OUT, { recursive: true, force: true });
FS.mkdirSync(OUT, { recursive: true });
const CASES_PER_FILE = 12;
const FILES = 40;
const NP = 6;

const manifest = [];
for (let f = 0; f < FILES; f++) {
  let fns = "";
  for (let c = 0; c < CASES_PER_FILE; c++) {
    const doc = genJson(ri(1, 3));
    const probes = Array.from({ length: NP }, () => {
      const key = pick(KEYS);
      const mode = chance(0.5) ? "str" : (chance(0.8) ? "int" : "dot2");
      return { key, mode };
    });
    const fname = `c${c}`;
    const params = Array.from({ length: NP }, (_, i) => `k${i}: string`).join(", ");
    const body = probes.map((p, i) => {
      if (p.mode === "str") {
        return `  r = r + "|" + (near.jsonGetStr(k${i}) ?? "~") + (near.jsonGetStr(${JSON.stringify(p.key)}) ?? "~") + (near.jsonGetStr(k${i}, near.storageGet("doc") ?? "{}") ?? "~");`;
      }
      if (p.mode === "int") {
        return `  r = r + "|" + (near.jsonGetInt(k${i}) ?? -1) + "," + (near.jsonGetInt(${JSON.stringify(p.key)}) ?? -1) + "," + (near.jsonGetInt(k${i}, near.storageGet("doc") ?? "{}") ?? -1);`;
      }
      // dot2: dot-path against the stored doc only
      const path = p.key.includes(".") ? p.key : p.key + ".x";
      return `  r = r + "|" + (near.jsonGetStr(${JSON.stringify(path)}, near.storageGet("doc") ?? "{}") ?? "~");`;
    }).join("\n");
    fns += `
export function ${fname}(${params}): string {
  near.storageSet("doc", ${JSON.stringify(doc)});
  let r = "s";
${body}
  return r;
}
`;
    const input = {};
    for (const p of probes) if (!p.key.includes(".")) input[p.key] = "7";
    manifest.push({
      file: `json_${String(f).padStart(3, "0")}`, method: fname,
      doc, probes, input, args: probes.map(p => p.key),
    });
  }
  FS.writeFileSync(PATH.join(OUT, `json_${String(f).padStart(3, "0")}.ts`), fns);
}
FS.writeFileSync(PATH.join(__dirname, "corpus", "json_manifest.json"), JSON.stringify(manifest));
console.log(JSON.stringify({ files: FILES, cases: manifest.length }));
