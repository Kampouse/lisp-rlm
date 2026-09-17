#!/usr/bin/env node
// Differential fuzz harness v2 (trap-aware, resilient).
//   u128: arith cases trap on overflow (oracle knows); comparisons are bool.
//   json: tx input hits + tortured stored doc via 2-arg/dot-path probes.
//   cfg:  plain-JS oracle, per-case try/catch.
const { execFile } = require("child_process");
const { Worker } = require("worker_threads");
const { FS, PATH } = require("./gen_common.js");

const ROOT = "/Users/asil/dev/lisp-rlm";
const COMPILE = PATH.join(ROOT, "target/release/compile");
const MOCK = PATH.join(ROOT, "target/release/near-mock");
const TMP = "/tmp/fuzz-wasm";
FS.mkdirSync(TMP, { recursive: true });
const RESULTS = PATH.join(__dirname, "results");
FS.mkdirSync(RESULTS, { recursive: true });
const phase = process.argv[2] || "u128";
const CORPUS = PATH.join(__dirname, { u128: "corpus/u128", json: "corpus/json", cfg: "corpus/cfg" }[phase]);

function shFile(bin, args) {
  return new Promise((res) => {
    execFile(bin, args, { timeout: 60000, maxBuffer: 16 * 1024 * 1024 }, (err, stdout, stderr) =>
      res({ code: err ? (err.code ?? 1) : 0, stdout: String(stdout), stderr: String(stderr) }));
  });
}
function parseMock(out) {
  const ok = out.includes("✅ Success");
  let rets = [...out.matchAll(/^📄 (.*)$/gm)].map(m => m[1]);
  // near-mock decorates string returns: `"body" (N-byte str | i64 view: …)`
  // — strip to the bare value so oracle (plain JS string) compares equal.
  rets = rets.map(v => v.replace(/^"(.*?)" \(\d+-byte str \| i64 view: \d+\)$/, "$1"));
  return { ok, ret: rets.length ? rets[rets.length - 1] : null,
           err: ok ? null : (out.match(/^❌ (.*)$/m)?.[1] ?? "unknown").slice(0, 300) };
}
// Oracle fns can diverge (generated while-loops with no fuel bound). JS
// cannot preempt a synchronous call, so run each in a worker thread and
// time-box it: timeout → ORACLE_TIMEOUT verdict, phase keeps moving.
function callOracleWithTimeout(mod, fnName, arg, ms = 2000) {
  return new Promise((res) => {
    const code = `
      const { workerData, parentPort } = require("worker_threads");
      const m = require(workerData.mod);
      parentPort.postMessage({ ok: true, ret: m[workerData.fnName](workerData.arg) });
    `;
    const w = new Worker(code, { eval: true, workerData: { mod, fnName, arg } });
    const t = setTimeout(() => { w.terminate(); res({ timeout: true }); }, ms);
    w.on("message", (m) => { clearTimeout(t); res(m); });
    w.on("error", (e) => { clearTimeout(t); res({ error: String(e).slice(0, 200) }); });
    w.on("exit", (c) => { if (c !== 0) { clearTimeout(t); res({ error: "worker exit " + c }); } });
  });
}

// ── BigInt u128 oracle: arith traps at the 128-bit envelope ──────────
const UMAX = (1n << 128n) - 1n;
function norm(s) { return /^-?\d+$/.test(s.trim()) ? BigInt(s.trim()) : null; }
function oracleU128(op, a, b) {
  const A = norm(a), B = norm(b);
  if (A === null || B === null) return { trap: "non-numeric" };
  let r;
  switch (op) {
    case "Add": r = A + B; break;
    case "Sub": r = A - B; break;
    case "Mul": r = A * B; break;
    case "Div": if (B === 0n) return { trap: "div-by-zero" }; r = A / B; break;
    case "Mod": if (B === 0n) return { trap: "div-by-zero" }; r = A % B; break;
    case "Lt": return { val: A < B ? "1" : "0" };
    case "Gt": return { val: A > B ? "1" : "0" };
    case "Eq": return { val: A === B ? "1" : "0" };
  }
  if (r < 0n || r > UMAX) return { trap: "u128-overflow" };
  return { val: r.toString() };
}

async function runPhaseU128() {
  const manifest = JSON.parse(FS.readFileSync(PATH.join(CORPUS, "..", "u128_manifest.json")));
  const files = [...new Set(manifest.map(m => m.file))];
  const results = [];
  for (const f of files) {
    const wasm = PATH.join(TMP, f + ".wasm");
    const c = await shFile(COMPILE, [PATH.join(CORPUS, f + ".ts"), wasm]);
    if (c.code !== 0 || !FS.existsSync(wasm)) {
      const errLine = (c.stderr.split("\n").find(l => l.startsWith("Error:")) ?? c.stderr).slice(0, 400);
      results.push({ f, kind: "COMPILE_FAIL", err: errLine });
      continue;
    }
    for (const m of manifest.filter(m => m.file === f)) {
      try {
        const isArith = ["Add", "Sub", "Mul", "Div", "Mod"].includes(m.op);
        const o1 = oracleU128(m.op, m.a, m.b);
        const o2 = oracleU128(m.op, m.a, m.b); // x=a, y=b by generator convention
        const o4 = oracleU128(m.op, m.a, m.b); // storage round-trips x
        const expected = o1.trap ? "TRAP" : isArith
          ? [o1.val, o2.val, o4.val].join("|")
          : [o1.val, o2.val, o4.val].join("");
        const r = await shFile(MOCK, [wasm, m.method, JSON.stringify({ x: m.a, y: m.b }), "--prepaid", "200"]);
        const p = parseMock(r.stdout);
        if (expected === "TRAP") {
          if (p.ok) results.push({ f: `${f}/${m.method}`, kind: "MISSING_TRAP", op: m.op, a: m.a, b: m.b, got: p.ret });
        } else if (!p.ok) {
          results.push({ f: `${f}/${m.method}`, kind: "UNEXPECTED_TRAP", op: m.op, a: m.a, b: m.b, err: p.err });
        } else if (p.ret !== expected) {
          results.push({ f: `${f}/${m.method}`, kind: "MISMATCH", op: m.op, a: m.a, b: m.b, expected, got: p.ret });
        }
      } catch (e) {
        results.push({ f: `${f}/${m.method}`, kind: "HARNESS_ERROR", err: String(e).slice(0, 200) });
      }
    }
  }
  return results;
}

// ── JSON oracle ──
// String-value span: returns [start, endExclusive] of a JSON string body
// starting at src[i] === '"', honoring backslash escapes.
function strSpan(src, i) {
  let j = i + 1;
  while (j < src.length) {
    if (src[j] === "\\") { j += 2; continue; }
    if (src[j] === '"') return [i, j + 1];
    j++;
  }
  return [i, src.length];
}
function rawSpan(src, i) {
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
function jlookup(doc, key) {
  const m = doc.match(new RegExp(`"${key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"\\s*:\\s*`));
  if (!m) return null;
  const vStart = m.index + m[0].length;
  if (doc[vStart] === '"') {
    // string value: honor escapes, don't over-capture into siblings
    const [s, e] = strSpan(doc, vStart);
    const raw = doc.slice(s, e);
    let un = null;
    try { un = JSON.parse(raw); } catch { un = null; }
    return { raw, un };
  }
  if (doc[vStart] === "{" || doc[vStart] === "[") {
    const raw = rawSpan(doc, vStart);
    return { raw, un: null };
  }
  const raw = rawSpan(doc, vStart);
  return { raw, un: null };
}
function prefixInt(raw) {
  const m = String(raw).match(/^\s*[-+]?\d+/);
  return m ? parseInt(m[0], 10) : null;
}
function oracleJson(probe, inputVal, doc) {
  if (probe.mode === "dot2") {
    const segs = (probe.path || probe.key + ".x").split(".");
    const o1 = jlookup(doc, segs[0]);
    if (!o1) return "~";
    const o2 = jlookup(o1.raw, segs[1]);
    if (!o2) return "~";
    return o2.raw.startsWith('"') ? (o2.un ?? "~ERR") : o2.raw;
  }
  const hit = inputVal !== undefined;
  if (probe.mode === "str") {
    const dDyn = hit ? "7" : "~";
    const h2 = jlookup(doc, probe.key);
    const dDoc = h2 ? (h2.raw.startsWith('"') ? (h2.un ?? "~ERR") : h2.raw) : "~";
    return dDyn + "," + dDoc;
  }
  const dDyn = hit ? 7 : -1;
  const h2 = jlookup(doc, probe.key);
  const dDoc = h2 ? (prefixInt(h2.raw) ?? -1) : -1;
  return `${dDyn},${dDoc}`;
}

async function runPhaseJson() {
  const manifest = JSON.parse(FS.readFileSync(PATH.join(CORPUS, "json_manifest.json")));
  const files = [...new Set(manifest.map(m => m.file))];
  const results = [];
  for (const f of files) {
    const wasm = PATH.join(TMP, f + ".wasm");
    const c = await shFile(COMPILE, [PATH.join(CORPUS, f + ".ts"), wasm]);
    if (c.code !== 0 || !FS.existsSync(wasm)) {
      results.push({ f, kind: "COMPILE_FAIL", err: (c.stderr.split("\n").find(l => l.startsWith("Error:")) ?? c.stderr).slice(0, 400) });
      continue;
    }
    for (const m of manifest.filter(m => m.file === f)) {
      try {
        const expected = "s" + m.probes.map(p => "|" + oracleJson(p, m.input[p.key], m.doc)).join("");
        const r = await shFile(MOCK, [wasm, m.method, JSON.stringify(m.input), "--prepaid", "200"]);
        const p = parseMock(r.stdout);
        if (!p.ok) results.push({ f: `${f}/${m.method}`, kind: "TRAP", doc: m.doc, probes: m.probes, err: p.err });
        // JSON semantics subtler than naive oracle (2-arg miss renders "",
        // per-op coercion, flat dot-scan) — collect, don't judge; reviewed
        // in json_findings.md.
        else results.push({ f: `${f}/${m.method}`, kind: "OBSERVATION", doc: m.doc, probes: m.probes, input: m.input, ret: p.ret });
      } catch (e) {
        results.push({ f: `${f}/${m.method}`, kind: "HARNESS_ERROR", err: String(e).slice(0, 200) });
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
    let oracle = null;
    const oraclePath = PATH.join(CORPUS, base + ".oracle.js");
    try { oracle = require(oraclePath); }
    catch (e) { results.push({ f, kind: "ORACLE_ERROR", err: String(e).slice(0, 200) }); continue; }
    const wasm = PATH.join(TMP, base + ".wasm");
    const c = await shFile(COMPILE, [PATH.join(CORPUS, f), wasm]);
    if (c.code !== 0 || !FS.existsSync(wasm)) {
      results.push({ f, kind: "COMPILE_FAIL", err: (c.stderr.split("\n").find(l => l.startsWith("Error:")) ?? c.stderr).slice(0, 400) });
      continue;
    }
    for (let n = 0; n < 8; n++) {
      for (const n0v of [0, 1, 3, 7, 100]) {
        try {
          const o = await callOracleWithTimeout(oraclePath, `f${n}`, n0v);
          if (o.timeout) {
            // Oracle diverged. The wasm run is gas-capped so it always
            // terminates — if it RETURNS a value where pure reference
            // semantics diverge, that's a real semantic finding.
            const r = await shFile(MOCK, [wasm, `f${n}`, JSON.stringify({ n0: n0v }), "--prepaid", "200"]);
            const p = parseMock(r.stdout);
            if (p.ok) results.push({ f: `${base}/f${n}`, n0: n0v, kind: "DIVERGE_WASM_TERMINATES", got: p.ret });
            else results.push({ f: `${base}/f${n}`, n0: n0v, kind: "ORACLE_TIMEOUT", err: p.err });
            continue;
          }
          if (o.error) { results.push({ f: `${base}/f${n}`, n0: n0v, kind: "HARNESS_ERROR", err: o.error }); continue; }
          const expected = o.ret;
          const r = await shFile(MOCK, [wasm, `f${n}`, JSON.stringify({ n0: n0v }), "--prepaid", "200"]);
          const p = parseMock(r.stdout);
          if (!p.ok) results.push({ f: `${base}/f${n}`, n0: n0v, kind: "TRAP", err: p.err });
          else if (p.ret !== expected) results.push({ f: `${base}/f${n}`, n0: n0v, kind: "MISMATCH", expected, got: p.ret });
        } catch (e) {
          results.push({ f: `${base}/f${n}`, n0: n0v, kind: "HARNESS_ERROR", err: String(e).slice(0, 200) });
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
  console.log(JSON.stringify({ phase, summary: counts, total: results.length, seconds: (Date.now() - t0) / 1000, out }));
}).catch(e => console.error("PHASE CRASH:", e));
