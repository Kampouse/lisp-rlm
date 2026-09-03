# Browser Playground Rebuild Report

**Date:** 2026-08-31
**Operator:** OpenClaw subagent (browser-rebuild)

## Commit

- `main` @ **7884db8** — `fix(wasm): near/promise_result arity guard` ✅ (meets the "7884db8 or newer" requirement; `git pull` confirmed "Already up to date")
- No commits pushed, nothing deployed, main untouched beyond pull.

## pkg consumption convention (discovered, differs from task brief)

The task suggested `--out-dir ../../web-app/src/pkg`, but the repo convention is different:

- The real playground app lives at **`crates/browser-compiler/web-app/`** (not repo-root `web-app/` — that doesn't exist; `crates/web-app/` is a nearly-empty stub with only `public/`).
- `src/lib/compiler.ts` imports the pkg via:
  `import init, { compile_p1, compile_p2, compile_p2_core, compile_pure, compile_ts, ts_to_lisp, disassemble_wasm } from '../../public/wasm/lisp_rlm_browser.js'`
  → resolves to `crates/browser-compiler/web-app/public/wasm/lisp_rlm_browser.js`.
- `public/_headers` also serves `/wasm/*` with `Cache-Control: no-cache`.

**Command actually used** (from `crates/browser-compiler/`):
```
wasm-pack build --target web --release --out-dir ./web-app/public/wasm --out-name lisp_rlm_browser
```

## 1. wasm pkg build — ✅ SUCCESS

- `rustup`: wasm32-unknown-unknown already installed; wasm-pack 0.14.0.
- Result: `Done in 3m 20s` (release, wasm-opt applied). Compiles clean apart from pre-existing warnings in `lisp-rlm-wasm` (40 warnings, e.g. non-snake-case `Str` in `src/ts_frontend.rs:2375`).
- Output: 5 files in `crates/browser-compiler/web-app/public/wasm/` (js, d.ts, wasm, wasm.d.ts, package.json).
- **`lisp_rlm_browser_bg.wasm` = 9,730,520 bytes (~9.3 MB).**
- Glue-export sanity check: all 7 named functions imported by `compiler.ts` (`compile_p1`, `compile_p2`, `compile_p2_core`, `compile_pure`, `compile_ts`, `ts_to_lisp`, `disassemble_wasm`) plus `init` are present in the generated JS.

**Note / fix during build:** the brief's suggested `--out-dir ../../web-app/public/wasm` resolves relative to the *crate dir* → repo-root `web-app/` (which wasm-pack auto-created). I caught this in the wasm-pack log ("Your wasm pkg is ready to publish at /Users/asil/dev/lisp-rlm/web-app/public/wasm") and `mv`'d the output to the correct `crates/browser-compiler/web-app/public/wasm/`, then removed the stray root `web-app/` directory. No rebuild needed.

## 2. browser-compiler crate tests — ✅ PASS (0 tests)

- Real crate name (from `Cargo.toml`): **`lisp-rlm-browser`**.
- `cargo test -p lisp-rlm-browser` → compiled OK, `0 passed; 0 failed` for both unit tests and doc-tests. The crate is a thin wasm-bindgen shim (cdylib+rlib) with no tests of its own; the compiler logic it wraps (`lisp-rlm-wasm`) is exercised by the workspace suite (run separately today per instructions).

## 3. Web app build — ✅ SUCCESS

- npm used (`package-lock.json` present and authoritative; `npm ci` clean install, then `npm run build` = `tsc && vite build`).
- **tsc: passed** (no type errors). **Vite (rolldown) build: `✓ built in 5.80s`.**
- One non-fatal warning: `index` chunk is 3.9 MB > 500 kB warning limit (Monaco editor + app in one chunk) — pre-existing app structure, not touched.
- Output dir: `crates/browser-compiler/web-app/dist/` — **105 files, 23 MB total**. `public/wasm/` correctly copied to `dist/wasm/` (all 5 pkg files present).

### Top 5 dist assets by size

| Asset | Size (KB) |
|---|---|
| `dist/wasm/lisp_rlm_browser_bg.wasm` | 10,304 |
| `dist/assets/ts.worker-DUs6XC43.js` | 6,736 |
| `dist/assets/index-DE3hyeec.js` | 3,816 (gzip 1,010) |
| `dist/assets/css.worker-BERHMMy7.js` | 1,032 |
| `dist/assets/html.worker-DRK-BLhy.js` | 704 |

## Fixes made

- None to source. One operational correction: pkg output moved from the wrongly-resolved repo-root `web-app/public/wasm` to `crates/browser-compiler/web-app/public/wasm` (see note above). No repo files modified — `git status` unchanged apart from pre-existing untracked `--out` file and `vendor/` dir that predate this task.

## Blockers

- **None.** Pkg builds, crate compiles/tests green, web app type-checks and bundles, wasm wired into dist.

## Repro commands

```bash
cd crates/browser-compiler
wasm-pack build --target web --release --out-dir ./web-app/public/wasm --out-name lisp_rlm_browser
cargo test -p lisp-rlm-browser
cd web-app && npm ci && npm run build
```
