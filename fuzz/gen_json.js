// JSON torture v4. API reality (verified by probe):
//   - dynamic keys: tx-input lookups ONLY (jsonGetStr(k)/jsonGetInt(k))
//   - provided-buffer reads (2-arg): literal keys only — jsonGetStr("k", doc),
//     jsonGetInt("n", doc), dot-paths "a.b" — via a doc LOCAL from storage
// Oracle: str part = "dyn,doc", int part = "dyn,doc", dot2 part = "doc".
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
        return `  r = r + "|" + (near.jsonGetStr(k${i}) ?? "~") + "," + (near.jsonGetStr(${JSON.stringify(p.key)}, doc) ?? "~");`;
      }
      if (p.mode === "int") {
        return `  r = r + "|" + (near.jsonGetInt(k${i}) ?? -1) + "," + (near.jsonGetInt(${JSON.stringify(p.key)}, doc) ?? -1);`;
      }
      const path = p.key.includes(".") ? p.key : p.key + ".x";
      return `  r = r + "|" + (near.jsonGetStr(${JSON.stringify(path)}, doc) ?? "~");`;
    }).join("\n");
    fns += `
export function ${fname}(${params}): string {
  near.storageSet("doc", ${JSON.stringify(doc)});
  let doc = near.storageGet("doc") ?? "{}";
  let r = "s";
${body}
  return r;
}
`;
    const input = {};
    for (const p of probes) if (!p.key.includes(".")) input[p.key] = "7";
    manifest.push({ file: `json_${String(f).padStart(3, "0")}`, method: fname, doc, probes, input });
  }
  FS.writeFileSync(PATH.join(OUT, `json_${String(f).padStart(3, "0")}.ts`), fns);
}
FS.writeFileSync(PATH.join(__dirname, "corpus", "json", "json_manifest.json"), JSON.stringify(manifest));
console.log(JSON.stringify({ files: FILES, cases: manifest.length }));
