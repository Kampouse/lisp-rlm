#!/usr/bin/env node
// Differential fuzz harness.
//   Phase u128: compile each u128_*.ts → wasm, run every entrypoint with the
//     edge pair as args, compare to a BigInt oracle (4 result forms each).
//   Phase json: same, comparing to a semantics oracle (raw-span/unescape rules).
//   Phase cfg:  compile random control-flow programs, compare to plain-JS
//     oracle. Args: n0 number.
// Usage: node run_fuzz.js <phase> [--workers N] [--only GLOB]
const { execFile, exec } = require("child_process");
const { FS, PATH, U128_LIST } = require("./gen_common.js");

const ROOT = "/Users/asil/dev/lisp-rlm";
const COMPILE = PATH.join(ROOT, "target/release/compile");
const MOCK = PATH.join(ROOT, "target/release/near-mock");
const TMP = "/tmp/fuzz-wasm";
FS.mkdirSync(TMP, { recursive: true });
const RESULTS = PATH.join(__dirname, "results");
FS.mkdirSync(RESULTS, { recursive: true });
const phase = process.argv[2] || "u128";

const dirOf = { u128: "corpus/u128", json: "corpus/json", cfg: "corpus/cfg" }[phase];
const CORPUS = PATH.join(__dirname, dirOf);

// ── small helpers ─────────────────────────────────────────────────────
function sh(cmd, opts = {}) {
  return new Promise((res) => {
    exec(cmd, { timeout: 60000, maxBuffer: 16 * 1024 * 1024, ...opts }, (err, stdout, stderr) =>
      res({ code: err ? (err.code ?? 1) : 0, stdout: String(stdout), stderr: String(stderr) }));
  });
}
function shFile(bin, args) {
  return new Promise((res) => {
    execFile(bin, args, { timeout: 60000, maxBuffer: 16 * 1024 * 1024 }, (err, stdout, stderr) =>
      res({ code: err ? (err.code ?? 1) : 0, stdout: String(stdout), stderr: String(stderr) }));
  });
}

// Parse near-mock output: "✅ Success" / "❌ err" + "📄 return"
function parseMock(out) {
  const ok = out.includes("✅ Success");
  const rets = [...out.matchAll(/^📄 (.*)$/gm)].map(m => m[1]);
  return { ok, ret: rets.length ? rets[rets.length - 1] : null,
           err: ok ? null : (out.match(/^❌ (.*)$/m)?.[1] ?? "unknown") };
}

// ── BigInt u128 oracle ────────────────────────────────────────────────
const UMAX = (1n << 128n) - 1n;
function norm(s) { // normalize decimal string input, reject junk
  if (!/^-?\d+$/.test(s.trim())) return null;
  return BigInt(s.trim());
}
function oracleU128(op, a, b) {
  const A = norm(a), B = norm(b);
  if (A === null || B === null) return { trap: "non-numeric operand" };
  let r;
  switch (op) {
    case "Add": r = A + B; break;
    case "Sub": r = A - B; break;
    case "Mul": r = A * B; break;
    case "Div": if (B === 0n) return { trap: "divide by zero" }; r = A / B; break;
    case "Mod": if (B === 0n) return { trap: "divide by zero" }; r = A % B; break;
    case "Lt": r = A < B ? 1n : 0n; break;
    case "Gt": r = A > B ? 1n : 0n; break;
    case "Eq": r = A === B ? 1n : 0n; break;
  }
  const wrapped = r < 0n || r > UMAX;
  const mod = ((r % (UMAX + 1n)) + (UMAX + 1n)) % (UMAX + 1n);
  return { val: mod.toString(), wrapped };
}

// ── JSON semantics oracle ─────────────────────────────────────────────
function rawSpan(src, i) { // value span starting at i (i points at value)
  let d = 0, inStr = false;
  for (let j = i; j < src.length; j++) {
    const c = src[j];
    if (inStr) { if (c === "\\") j++; else if (c === '"') inStr = false; continue; }
    if (c === '"') { inStr = true; continue; }
    if (c === "{" || c === "[") d++;
    if (c === "}" || c === "]") { d--; if (d === 0 && src[i] !== '"') return src.slice(i, j + 1); }
    if (d === 0 && (c === "," || c === "}") && src[i] !== '"' && src[i] !== "{" && src[i] !== "[") {
      let e = j; while (e > i && src[e - 1] === " ") e--;
      return src.slice(i, e);
    }
  }
  return src.slice(i).replace(/\s+$/, "");
}
function jlookup(doc, key) { // top-level key → {raw, unescaped} | null
  const m = doc.match(new RegExp(`"${key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"\\s*:\\s*`));
  if (!m) return null;
  const vs = m.index + m[0].length;
  const raw = rawSpan(doc, vs);
  let un = raw;
  if (raw.startsWith('"')) {
    try { un = JSON.parse(raw); } catch { un = null; }
  }
  return { raw, un };
}
function prefixInt(raw) {
  const m = String(raw).match(/^\s*[-+]?\d+/);
  return m ? parseInt(m[0], 10) : null;
}
function oracleJson(probe, inputVal, doc) {
  // direct lookups hit the canonical input object; 2-arg forms hit the doc
  if (probe.mode === "dot2") {
    const [seg1, seg2] = probe.path.split(".");
    const o1 = jlookup(doc, seg1);
    if (!o1) return "~";
    const o2 = jlookup(o1.raw, seg2);
    if (!o2) return "~";
    return o2.raw.startsWith('"') ? (o2.un ?? "~ERR") : o2.raw;
  }
  const direct = probe.mode === "int" ? -1 : "~";
  const fromDoc = probe.mode === "int" ? -1 : "~";
  const hit = inputVal !== undefined;
  if (probe.mode === "str") {
    const d = direct === "~" && hit ? "7" : direct;
    const h2 = jlookup(doc, probe.key);
    const dv = h2 ? (h2.raw.startsWith('"') ? (h2.un ?? "~ERR") : h2.raw) : fromDoc;
    return d + dv;
  }
  const d = hit ? 7 : direct;
  const h2 = jlookup(doc, probe.key);
  const dv = h2 ? (prefixInt(h2.raw) ?? -1) : fromDoc;
  return `${d},${dv}`;
}

// ── phase runners ─────────────────────────────────────────────────────
async function runPhaseU128() {
  const manifest = JSON.parse(FS.readFileSync(PATH.join(CORPUS, "..", "u128_manifest.json")));
  const files = [...new Set(manifest.map(m => m.file))];
  const results = [];
  for (const f of files) {
    const tsPath = PATH.join(CORPUS, f + ".ts");
    const wasm = PATH.join(TMP, f + ".wasm");
    const c = await shFile(COMPILE, [tsPath, wasm]);
    if (c.code !== 0 || !FS.existsSync(wasm)) {
      results.push({ f, kind: "COMPILE_FAIL", stdout: c.stdout.slice(0, 2000), stderr: c.stderr.slice(0, 2000) });
      continue;
    }
    // one wasm → 60 entrypoints; distinct args per entrypoint
    const entries = manifest.filter(m => m.file === f);
    for (const m of entries) {
      const x = m.a, y = m.b;
      const o1 = oracleU128(m.op, x, y);
      const o2 = oracleU128(m.op, o1.trap ? "0" : o1.val, o1.trap ? "0" : y);
      const o3 = oracleU128(m.op, o1.trap ? "0" : o1.val, o2.trap ? "0" : o2.val);
      const o4 = oracleU128(m.op, x, o1.trap ? "0" : o1.val);
      const expected = o1.trap ? "TRAP" : [o1.val, o2.val, o3.val, o4.val].join("|");
      const r = await shFile(MOCK, [wasm, m.method, JSON.stringify({ x, y }), "--prepaid", "200"]);
      const p = parseMock(r.stdout);
      if (o1.trap) {
        if (p.ok) results.push({ f: `${f}/${m.method}`, kind: "MISSING_TRAP", op: m.op, a: x, b: y, got: p.ret });
      } else if (!p.ok) {
        results.push({ f: `${f}/${m.method}`, kind: "UNEXPECTED_TRAP", op: m.op, a: x, b: y, err: p.err, expected });
      } else if (p.ret !== expected) {
        results.push({ f: `${f}/${m.method}`, kind: "MISMATCH", op: m.op, a: x, b: y, expected, got: p.ret });
      }
    }
  }
  return results;
}

async function runPhaseJson() {
  const manifest = JSON.parse(FS.readFileSync(PATH.join(CORPUS, "json_manifest.json")));
  const files = [...new Set(manifest.map(m => m.file))];
  const results = [];
  for (const f of files) {
    const tsPath = PATH.join(CORPUS, f + ".ts");
    const wasm = PATH.join(TMP, f + ".wasm");
    const c = await shFile(COMPILE, [tsPath, wasm]);
    if (c.code !== 0 || !FS.existsSync(wasm)) {
      results.push({ f, kind: "COMPILE_FAIL", stdout: c.stdout.slice(0, 3000), stderr: c.stderr.slice(0, 3000) });
      continue;
    }
    for (const m of manifest.filter(m => m.file === f)) {
      const expected = "s" + m.probes.map(p => "|" + oracleJson(p, m.input[p.key], m.doc)).join("");
      const callArgs = JSON.stringify(m.input);
      const r = await shFile(MOCK, [wasm, m.method, callArgs, "--prepaid", "200"]);
      const p = parseMock(r.stdout);
      if (!p.ok) {
        results.push({ f: `${f}/${m.method}`, kind: "TRAP", doc: m.doc, probes: m.probes, err: p.err });
      } else if (p.ret !== expected) {
        results.push({ f: `${f}/${m.method}`, kind: "MISMATCH", doc: m.doc, probes: m.probes, expected, got: p.ret });
      }
    }
  }
  return results;
}

async function runPhaseCfg() {
  const results = [];
  const files = FS.readdirSync(CORPUS).filter(x => x.endsWith(".ts"));
  for (const f of files) {
    const base = f.replace(/\.ts$/, "");
    const tsPath = PATH.join(CORPUS, f);
    const oracle = require(PATH.join(CORPUS, base + ".oracle.js"));
    const wasm = PATH.join(TMP, base + ".wasm");
    const c = await shFile(COMPILE, [tsPath, wasm]);
    if (c.code !== 0 || !FS.existsSync(wasm)) {
      results.push({ f, kind: "COMPILE_FAIL", stdout: c.stdout.slice(0, 2000), stderr: c.stderr.slice(0, 2000) });
      continue;
    }
    const inputs = [0, 1, 3, 7, 100];
    for (let n = 0; n < 8; n++) {
      for (const n0v of inputs) {
        const expected = oracle[`f${n}`](n0v);
        const r = await shFile(MOCK, [wasm, `f${n}`, JSON.stringify({ n0: n0v }), "--prepaid", "200"]);
        const p = parseMock(r.stdout);
        if (!p.ok) {
          results.push({ f: `${base}/f${n}`, n0: n0v, kind: "TRAP", err: p.err });
        } else if (p.ret !== expected) {
          results.push({ f: `${base}/f${n}`, n0: n0v, kind: "MISMATCH", expected, got: p.ret });
        }
      }
    }
  }
  return results;
}

const t0 = Date.now();
const run = { u128: runPhaseU128, json: runPhaseJson, cfg: runPhaseCfg }[phase];
if (!run) { console.error("phase must be u128|json|cfg"); process.exit(1); }
run().then(results => {
  const out = PATH.join(RESULTS, `${phase}_${Date.now()}.json`);
  FS.writeFileSync(out, JSON.stringify(results, null, 1));
  const counts = {};
  for (const r of results) counts[r.kind] = (counts[r.kind] || 0) + 1;
  console.log(JSON.stringify({ phase, files_checked: results.length ? undefined : undefined,
    summary: counts, total_findings: results.length, seconds: (Date.now() - t0) / 1000, out }));
});
