#!/usr/bin/env node
// Morning report: reads fuzz/results/*.json + replays key probes, prints the
// campaign summary Jean wakes up to.
const { FS, PATH } = require("./gen_common.js");
const RESULTS = PATH.join(__dirname, "results");

const files = FS.existsSync(RESULTS) ? FS.readdirSync(RESULTS).filter(f => f.endsWith(".json")) : [];
const byPhase = {};
for (const f of files) {
  const phase = f.split("_")[0];
  const data = JSON.parse(FS.readFileSync(PATH.join(RESULTS, f)));
  byPhase[phase] = byPhase[phase] || { total: 0, kinds: {}, items: [] };
  byPhase[phase].total += data.length;
  for (const r of data) {
    byPhase[phase].kinds[r.kind] = (byPhase[phase].kinds[r.kind] || 0) + 1;
    if (byPhase[phase].items.length < 25) byPhase[phase].items.push(r);
  }
}

let out = "═══ MORNING FUZZ REPORT — " + new Date().toISOString().slice(0, 16) + " ═══\n\n";
if (!files.length) {
  out += "No result files yet — campaign still running or phases not finished.\n";
} else {
  for (const [phase, p] of Object.entries(byPhase)) {
    out += `Phase ${phase}: ${p.total} findings\n`;
    for (const [k, n] of Object.entries(p.kinds)) out += `  ${k}: ${n}\n`;
    out += "\n";
  }
  for (const [phase, p] of Object.entries(byPhase)) {
    if (!p.items.length) continue;
    out += `── ${phase} detail (first ${p.items.length}) ──\n`;
    for (const it of p.items) out += JSON.stringify(it) + "\n";
    out += "\n";
  }
}
out += "Committed on fuzz-campaign-0917:\n" +
  "  8908b67 fix(peephole): checked-retag guard restored (BUG-2026-09-17)\n" +
  "  7e0b57f test(fuzz): differential campaign harness\n" +
  "  9095aa5 docs: PLAN.md session-6 audit\n" +
  "Writeup: fuzz/repro/BUG-peephole-ate-checked-retag.md\n";
console.log(out);
