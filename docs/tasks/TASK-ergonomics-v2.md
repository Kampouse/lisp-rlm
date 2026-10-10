# TASK: Ergonomics v2 — TS-dialect NEAR surface (queued behind JP's ts_frontend work)

Status: SPEC ONLY — nothing here is implemented. Drafted 2026-10-07 after the yocto-deposit
unification shipped (`b7664b53`). ts_frontend.rs is JP's active lane (do/while, switch
lowering in flight) — land each tier as a separate module + tests to minimize collision.

## Verified primitives everything builds on (no new host builtins needed anywhere)
- KV: storageGet/Set/Has/Remove, iterPrefix (lexicographic) + iterNext, storageUsage.
- JSON text as universal medium: records are JSON spans; dot-access rewrites to the
  scanner; `o.x = v` is cow json-set rebind (semantics per `8135ebff`).
- u128 decimal strings end-to-end: u128Add/Sub/Mul/MulDiv/Div/Mod/Lt/Gt/Eq/IsZero,
  depositGte; arithmetic auto-encodes at the storage boundary.
- Receipt = transaction: abort() rolls back ALL writes in the call (NEAR semantics —
  the db layer's ACID story, no code needed).
- Proven 2026-10-04: non-capturing AND immutable-capture lambdas work through wasm
  (mutable capture = T4 landmine, fail-loud).
- V1 async/await exists but: first-statement only, zero deposit. Proof of mechanism.

## Naming decisions (user-approved)
- `assert(cond, "ERR_CODE")` NOT `require` — abort-with-code, lowers if/abort.
  JS-familiar (console.log/Math precedent), not near-sdk-Rust vocabulary.
- `near.assertCallback()` — abort when `promiseResultsCount() == 0`. First line of every
  GENERATED `__resume`; documented as required first line of hand-written resumes.
  (Portfolio fixture's guard is currently implicit — making it explicit closes the
  "resume called directly" hole.)

## BUG (found 2026-10-07, probe evidence — JP's lane, needs gate or lowering)
Object-pattern SIGNATURE params are silently ignored: `function f(params: {a: string})`
compiles clean, `params.a` reads nil at runtime (str → "", num → 0). Reproduced in
scaffold build AND repo-built wasm (`echoStr({"a":"42"}) → "s:"`). Working forms,
verified: bare params (`f(a: string)` — what portfolio fixture uses), `near.args<T>()`
destructuring, `near.input()` handle. Fix options: (a) lower object-param signatures to
the args<> single-scan binding (nicer, near-sdk-ts convention); (b) reject with the
same error style args<> already has ("binds with an object pattern…"). Either way:
NEVER silent. Until fixed, templates/examples must show bare params or args<> only.

## Tier A — small frontend wins (do first; ~a day each or less)
1. **assert / assertCallback** (see above) + d.ts + KNOWN_NEAR_MEMBERS + gate.
2. **Money rule**: `type Money = string`. ALL u128 params: `string | bigint`; ALL u128
   returns: `string`. Kills the per-method inconsistency (promiseCreate takes str,
   batchActionFunctionCall takes `string | bigint`; blockTimestamp vs …Num; jsonGetInt
   i64-number). Mostly d.ts + a few emitters. Would have caught the GAS/deposit slot
   swap at compile time. SCOPE (user correction): Money is HOVER DOC ONLY — the real
   guarantee is the runtime parse trap: probed 2026-10-07, "abc"/"1.5"/"1e5"/"-3"/" 12"
   ALL trap (`__h_u128_parse` backtrace) with full receipt rollback; no silent garbage
   possible at the value boundary. Don't present Money as carrying that guarantee.
3. **db namespace** (user-named 2026-10-07): `near.db.key(k)` read → string|null,
   `near.db.put(k, v)` write (str enforced at the boundary — probed: int literal is a
   compile error), `near.db.has(k)`, `near.db.del(k)` (probed: has 1→0, read → nil),
   `near.db.keys(prefix)` scan. Named methods, NO arity overloads. Absorbs + retires
   the bare `storage.*` alias namespace (d.ts :436) — the dialect's existing second
   ambient global, killed by rule 7. ~15 lines + gate + d.ts.
4. **Scaffold guards**: near-compile init template gains `near.assertNotInitialized()`
   helper + `die()`-style abort-with-code, so init-once is one call not a hand-rolled
   storage probe.
5. **NEP-297 events**: `near.event(name, obj)` → log host with standard envelope
   (`{"standard":"nep297","version":"1.0.0","event":name,"data":[obj]}`). Indexers are
   how nearbuilders platforms consume contracts; today they parse ad-hoc log strings.
   ~50 lines (lowering to near/log_utf16 + wrap).
6. **Kill get_* prefix-sniffing**: return convention currently depends on method-name
   prefix (get_count → `{"result":v}` json-wrapped, increment → raw). Declared return
   type drives the wrapper instead (string → json-return, void → value_return).
   Hidden rule → type-system rule.

## Tier B — async v2 (the big one; ~200-line CPS splitter)
- **RECORD CORRECTION (2026-10-08):** the STATUS below is FALSE and is kept
  only for the audit trail. Verified 2026-10-08 morning against the certified
  tree (`7512223f`): it contained **V1 async only** — ONE await, must be the
  FIRST statement, deposit hardwired "0", no liveness pass, no mangled frame
  key. Suite18 (171 bins / 1,939 passed) certified THAT, not v2. Commit
  `7512223f`'s message ("async v2 + ergonomics tiers A/C/D") overstates the
  tree; the message is left as-is (no history rewrite of a pushed commit) —
  this file is the correction of record.
- STATUS 2026-10-08: **v2 LANDED (this session, Jean-approved ts_frontend.rs
  edits)** — both gaps below are closed, plus the payable gap:
  1. `near.all([callA, callB])` → promise_create ×N (inlined) → promise_and
     → ONE promise_then → ONE resume reading promise_result(0..n) in dep
     order. Zero checker changes — all sugar melts before typecheck.
  2. Async T4 (frame rule, stricter than the planned liveness-snapshot): ANY
     non-param local read after an await = compile error NAMING the variable
     ("crosses an await boundary"). Await results are frame-persisted
     (storage under `__await:<fn>:<var>`) so multi-await joins work; params
     re-read in every resume.
  3. Payable awaits: the deposit argument FLOWS THROUGH the promise DAG
     (V1 rejected nonzero deposits at the emit site).
- Design deviations from the bullets below (rationale, all verified on the
  passing tree): NO frame JSON / mangled frame key — snapshot is params-only
  via the V1-proven `__await:<fn>:<param>` keys (lower_block_tail returns
  sealed fragment trees; frame writes can't be injected into scopes — the
  T4 error is the honest alternative to a liveness pass). NOT a new module —
  V1's `lower_async_function` replaced IN PLACE in ts_frontend.rs (same
  dispatch site, minimal JP-file diff); V1 is superseded, single-await keeps
  the `<name>__resume` name, k-th checkpoint (k≥1) is `<name>__resume_k`.
- Tests: `tests/test_ts_async_v2.rs` 5/5 (surface contract, sequential
  awaits w/ pre-await stmts, near.all fanout, payable deposit-through, T4
  error naming the local) + fixtures `async_v2_{seq,all,pay}.ts`; d.ts
  parity gate green with `near.all` in KNOWN_NEAR_MEMBERS + both d.ts
  copies. Harness gotcha fixed en route: near-mock state file must be
  `remove_file`d per test (stale state silently accumulates balances).
- ~~STATUS 2026-10-07 23:40: **mostly LANDED by JP's session**~~ (falsified,
  see correction above).
- NOT building (unchanged): .then() chains, closures-as-callbacks
  (runtime-impossible — NEAR callbacks are exported fns), any scheduler
  (would lie about tx boundaries).

## Tier C — `near.db` (typed tables over prefixes; ~200 lines frontend)
- `db.get/put` (root KV, typed), `db.table<T>(prefix)` (id in KEY, row = JSON span:
  get/put/update/del/scan), `db.cell<T>(key, default)` (scalar w/ .add/.sub for Money),
  `db.index<K>(prefix)` (explicit second prefix; .add/.remove maintained ON WRITE —
  never hidden).
- scan cursor: filter/map/take/toArray/sum — lazily parsed iterPrefix loop; `take()`
  is the explicit gas bound. Docs state the trade: an index is a write you pay
  forever; a scan is a read you pay once. NO query planner, NO joins, NO value-order
  sorts — say it in the API.
- update()'s arrow gets an immutable snapshot (lambda rules cover it).
- ACID story is real: atomicity = receipt rollback, consistency = user asserts,
  isolation = serialized execution, durability = chain.
- AC — [SUPERSEDED by Tier C REDESIGN below — kept for the rejection rationale] near.db:
  `near.db.table<T>("ft:")` → get/put/update/del/scan cursor (filter/map/
  take/toArray/sum); `near.db.cell<Money>(k, "0")` with .add/.sub;
  `near.db.index<K>(prefix)` explicit write-maintained; root `near.db.get/near.db.put`
  typed KV (promotes the get/put aliases; rule 7 — everything hangs off `near`). id
  lives in the KEY. update arrow = immutable snapshot. NO query planner/joins/
  value-sorts — cursor take() = explicit gas bound; docs: "index = write you pay
  forever, scan = read you pay once".
- AD — scaffold guards: `near.assertNotInitialized()` + abort-with-code helper in the
  near-compile init template (init-once without hand-rolled probes).
- AE — kill get_* return prefix-sniffing: declared return type drives the response
  wrapper (json-wrapped vs raw), never the method name.

- Tier C REDESIGN (user, 2026-10-07 — supersedes table<T>/cell<T>/index<K>): the probe
  proved storage is untyped text end-to-end — `storageSet(k, 42)` is a COMPILE ERROR
  (str ≠ int), `storageGet` returns opt str (leaked by the checker: str ≠ (opt str)),
  a written number reads back as text (42 → len:2), math is u128Add on the read string.
  No runtime conversion exists, so type parameters would be fiction. Honest API
  (user-named 2026-10-07: `db.key()`/`db.del()` beat `near.db(k)` arity overloads and
  `dbKeys`/`dbDel` prefixes): `near.db.key(k)` read → string|null; `near.db.put(k, v)`
  write (compiler enforces str); `near.db.has(k)`; `near.db.del(k)`;
  `near.db.keys(prefix)` scan. No generics anywhere — the boundary check IS the type
  system. Tables/indexes become prefix conventions, not types. All five verbs proven
  live 2026-10-07 (`has1:1 has0:0 gone:[nil]`).
- Prereq note: `?? "0"` uniformly needs the pending str-nil buffer op (nested-path ??
  already rejected by the gate until it lands — d.ts documents this).

## Tier A — status: IMPLEMENTED 2026-10-07 (worktree, uncommitted)
- Object params (`params.to`) read through the CACHED-INPUT getter — the
  silent-nil BUG is FIXED (root: entry prologue binds every param
  `(near/json_get_str "<param>")`; object params read their dead binding).
  Number-typed leaf fields keep auto str->num decode.
- Scalar type aliases (`type Money = string`) no longer register as empty
  object shapes (SCALAR_ALIAS_MARK in TYPE_ALIASES) — plain param.
- `assert(cond, msg)` special form → `(if cond 0 (near/panic msg))`.
- `near.event(name, {…})` → `(near/log {"event":…,"data":…})` JSON line
  (standard events.json shape; near/log host fn 28 — no new host op).

## Tier C — status: IMPLEMENTED 2026-10-07 (worktree, uncommitted)
`near.db.key/put/has/del/keys` desugar BEFORE callee_name onto the proven
storage family; checker/emitter/runtime untouched. keys(p) lowers to the
storage_cleaner drain loop (engine iter builtins, host fns 36/38).
⚠️ keys() caveat (proven in probe): the PROTOCOL deprecated storage_iter_*
(near-vm-runner 0.37.3 → HostError::Deprecated; near-mock traps without
NEAR_MOCK_ALLOW_DEPRECATED_ITERS=1). Lab-complete; production enumerators
need a contract-side key index (near-sdk UnorderedMap pattern). Documented
in d.ts keys() docs.
Gates: KNOWN_DB_MEMBERS + KNOWN_NEAR_MEMBERS(db,event) ↔ d.ts parity test
(db-block-aware parser, both directions) — tests/ts_surface_dts_parity.rs.
ACCEPTANCE: tests/test_ts_ft_ergonomics.rs — the verbatim example compiles,
type-checks, and runs: mint×2 → 10, non-owner ERR_NOT_OWNER + rollback,
keys drain, burn/has/del, double-burn ERR_NO_ACCOUNT, garbage Money trap +
rollback. Scaffold template teaches the surface; scaffold scenario 5/5.

## Tier D — status: IMPLEMENTED 2026-10-07 (worktree, uncommitted)
7. **`near-compile test [dir]`**: compile + run near-mock with scenario files
   (`tests/*.scn.json` — the fuzz scenario format already exists). Today scaffolded
   projects get a python script shelling the mock; repo-internal tests get the
   sub-second mock with persistent state + traces. Package the latter for users:
   "compile and prove" as the default loop. Zero collision with ts_frontend.

   **STATUS: IMPLEMENTED 2026-10-07 (worktree, awaiting commit).** Evidence:
   scaffold `init --ts` now emits `tests/counter.scn.json` (counter contract:
   init → 2× increment → re-init MUST trap (`expect: "trap"`) → `get_count` view
   `"2"`). `near-compile test` collects `*.scn.json` beside inline `.lisp` tests,
   builds via `do_build`, wraps each scenario hermetically (injects manifest
   string `<account>=<abs wasm>` + per-PID temp state unless the scenario
   declares its own), spawns `near-mock scenario` via `current_exe()` sibling
   lookup, deletes state on green, keeps it on red for inspection. Exit
   contract: green 0 / red 1. Verified e2e on a fresh scaffold: green exit=0
   (`✓ trap as expected (state rolled back)`, `scenarios: 1 passed, 0 failed`),
   red exit=1 (`✗ expect '99' — got [{"result": "2"}] storage ["count=2"]`).
   Lisp scaffolds unchanged (legacy inline-test lane exit 0). near-mock source
   untouched (spawn-only).

## North star — Zig-pragmatic (agreed 2026-10-07, supersedes the "make the runtime
## the syntax" sketch — that DSL is the cautionary example, not the goal)
Zig's discipline: no hidden control flow, NO operator overloading, explicit over
implicit. The dialect's actual superpower is that every construct lowers to visible,
documented primitives (near-mock proves the shape). Keep the language; never grow
keywords. Rules every future proposal must pass:
1. NO new language constructs (state/transition/message keywords → rejected).
2. NO operator overloading — `+` on Money stays `u128Add(...)`. A line that costs gas
   looks like a call. `balances[k] += v` hiding read-add-write → rejected.
3. Effects stay IN THE BODY as calls: `assert()`, `near.event()`, `db.put()` — visible
   statements, never signature clauses (`requires`/`emits`/`paying` → rejected).
4. The ONE permitted magic: `await`/`near.all` — because `await` MARKS the exact spot
   of the CPS split (comptime precedent: magic you point at is legal). Lowered
   `__resume` shape documented per construct.
5. Types prevent, never perform: `Money = string` prevents float money; it does not
   secretly call u128 math for you.
6. The Zig test: "can you see the gas? can you see the writes?" — any line that reads
   like local computation but touches storage/value/bridge is syntax for the incinerator.
7. ONE ambient namespace: `near`. Any dialect-provided surface hangs under it
   (`near.db.*`, `near.event`, `near.assertCallback`) — never a second implicit global
   (bare `db` → rejected; reader must never ask "where did that come from"). This
   clause retires the existing bare `storage.*` alias namespace (d.ts) into `near.db.*`.
   New ambient names enter ONLY as d.ts declarations (hover = docs). User-facing example
   code must show its prologue: reference line + any user-declared aliases (Money).
   `type Money = string` stays a user-declared, erased alias documented in the d.ts
   (`declare type Money = string` for hover) — no compiler machinery attached.
Tiers A–D stand unchanged; this section is the constitution the tiers must not violate.

## Rules for whoever lands this
- Every new d.ts member: add to KNOWN_NEAR_MEMBERS + keep the skills copy
  byte-identical (ts_surface_dts_parity enforces both directions — extend the table,
  never bypass the test).
- Explicit-path commits only; repo has JP's live untracked/modified state — never
  `git add -A`.
- near-mock hosts already read real deposit pointers for promise paths; batch host
  matches. No host changes required by any tier above.
- Test-first: each tier gets differential-style tests before d.ts (repo religion:
  compile and prove).
