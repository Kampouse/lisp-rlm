# lisp-rlm — Gap Tracker

**Last verified:** 2026-07-05 (source audit of 86 host functions, all `call_*.rs` modules)

> Generated from direct source audit, not memory. Old GAPS.md backed up to `GAPS.md.bak`.

---

## ✅ Implemented & Working

### Core Language
- [x] Tagged i64 values (3-bit tags: Num, Bool, FnRef, Closure, Nil, Str, Array)
- [x] Arithmetic: `+`, `-`, `*`, `/`, `mod`, `abs`, `min`, `max`, `isqrt`
- [x] Comparison: `<`, `>`, `<=`, `>=`, `=`, `!=`
- [x] Logic: `and`, `or`, `not`
- [x] Control flow: `if`/`else`, `begin`/`progn`, `while`, `for`, `loop`/`recur`
- [x] Bindings: `let`, `let*`, `set!`
- [x] Bitwise: `band`, `bor`, `bnot`, `shl`, `shr`, `clz`, `ctz`, `popcnt`, `bit_set/get/clr`
- [x] Wrap arithmetic: `wrap-add`, `wrap-sub`, `wrap-mul` (overflow-safe)
- [x] Type predicates: `number?`, `string?`, `bool?`, `nil?`, `list?`, `zero?`
- [x] `muldiv` (intermediate precision)
- [x] `cond` (multi-branch)
- [x] `assert`, `assert-equal`, `assert-true`, `assert-raises`

### Functions
- [x] `(define (name params...) body)` — function definitions
- [x] Multi-expression bodies (implicit begin)
- [x] `(export "name" func view_only)` — NEAR view/mutable exports
- [x] Gas metering (depth counter + eval budget, 1B op cap)
- [x] Function args via `env.input` / `read_register` / JSON
- [x] Named function calls (USER_BASE dispatch)
- [x] Dynamic dispatch (closure/fnref runtime resolution)
- [x] Self-passing call (Y-combinator pattern for recursion)

### Memory
- [x] `(memory N)` — declare pages
- [x] `i64.load` / `i64.store` — direct memory access
- [x] `mem-get` / `mem-set!` (i64), `mem-get8` / `mem-set8!` (byte via masked I64 word ops)
- [x] `store_i64` / `load_i64` — convenience memory helpers
- [x] `malloc` — runtime heap allocation (bump allocator at HEAP_START=200K)
- [x] Protected memory regions (guard against overwriting buffers/handles)
- [x] Handle table (256 entries × 16 bytes for struct access)

### Lists & Data Structures
- [x] Runtime lists: `list`, `car`, `cdr`, `cons`, `vec-push`, `vec-nth`, `vec-length`
- [x] Stdlib: `map`, `filter`, `find`, `member`, `take`, `drop`, `reverse`, `append`, `zip`, `range`, `length`, `nth`
- [x] Dict: `dict`, `dict/get`, `dict/set`, `dict/has?`, `dict/keys`, `dict/vals`
- [x] Inline HOF: `hof/map`, `hof/filter`, `hof/reduce` (compile-time lambda inlining)

### Strings
- [x] String literals (tagged ptr|len<<32)
- [x] `str-cat` / `str-concat` / `string-append`
- [x] `str-slice` / `substring`
- [x] `str-len` / `string-length`
- [x] `str-contains` / `str-contains-byte`
- [x] `str-ptr` (get raw pointer)
- [x] `to-string` / `itoa` (i64 → decimal string)
- [x] `bytes-to-u32` (byte parsing)
- [x] `store-bytes` / `load-bytes`

### NEAR Host Functions (86/86 mapped)
- [x] **Registers:** read_register(0), register_len(1), write_register(2)
- [x] **Context:** current_account_id(3), signer_account_id(4), signer_account_pk(5), predecessor_account_id(6), input(7)
- [x] **Blockchain:** block_index(8), block_timestamp(9), epoch_height(10), storage_usage(11)
- [x] **Economics:** account_balance(12), account_locked_balance(13), attached_deposit(14), prepaid_gas(15), used_gas(16)
- [x] **Storage:** storage_write(17), storage_read(18), storage_remove(19), storage_has_key(20)
- [x] **Crypto:** sha256(21), keccak256(22), random_seed(23), ed25519_verify(24), keccak512(52), ripemd160(53), ecover(54), p256_verify(55)
- [x] **Misc:** value_return(25), panic(26), panic_utf8(27), log_utf8(28), log_utf16(29)
- [x] **Promises:** promise_create(30), promise_then(31), promise_and(32), promise_results_count(33), promise_result(34), promise_return(35)
- [x] **Iterators:** storage_iter_prefix(36), storage_iter_range(37), storage_iter_next(38)
- [x] **Promise batch:** create(39), then(40), create_account(41), deploy_contract(42), function_call(43), transfer(44), stake(45), add_key_full(46), add_key_func(47), delete_key(48), delete_account(49)
- [x] **Global contracts:** deploy_contract(50), current_code_hash(51), deploy_global(75-76), use_global(77-78)
- [x] **Alt BN128:** g1_multiexp(56), g1_sum(57), pairing_check(58)
- [x] **BLS12-381:** p1_sum(59), p2_sum(60), g1_multiexp(61), g2_multiexp(62), map_fp_to_g1(63), map_fp2_to_g2(64), pairing_check(65), p1_decompress(66), p2_decompress(67)
- [x] **Advanced promises:** set_refund_to(68), state_init(69-70), set_state_init_data_entry(71), current_contract_code(72), refund_to_account_id(73), function_call_weight(74), transfer_to_gas_key(79), add_gas_key_full(80), add_gas_key_func(81), yield_create(82), yield_resume(83)
- [x] **Validator:** validator_stake(84), validator_total_stake(85)

### Higher-Level Builtins
- [x] **Composite KV:** `near/kv` (write), `near/kv-get` (read) — composite key builder
- [x] **Deposit helper:** `near/deposit-gte` (1 NEAR check with literal lo/hi)
- [x] **Signer buffer:** `near/signer_to_buf`, `near/write_amount`
- [x] **JSON:** `near/json_get_int`, `near/json_get_str`, `near/json_get_u128`, `near/json_return_int`, `near/json_return_str`
- [x] **Borsh (partial):** `borsh-serialize`, `borsh-deserialize` (see gaps below)
- [x] **u128:** `from_yocto`, `from_str`, `from_i64`, `new`, `to_i64`, `to_str`, `add`, `sub`, `mul`, `div`, `eq`, `lt`, `is_zero`, `load`, `load_high`, `store`, `store_storage`, `load_storage`
- [x] **FP64:** `fp64/set_int`, `fp64/get_int`, `fp64/get_frac`, `fp64/mul`, `fp64/div`, `fp64/sqrt` (Q64.64)
- [x] **DeFi math:** `tick_to_price`, `tick_to_price64`, `price_to_tick`, `price64_to_tick`, `sqrt`, `liq_amount0`, `liq_amount0_64`, `liq_amount1`, `liq_amount1_64`
- [x] **Logging:** `near/log` (string), `near/log "str" num` (combined), `near/log_num`, `near/log_utf16`

### Tooling
- [x] WASM validation on compile (wasmparser + function-name error mapping)
- [x] Type checking (lightweight pre-pass)
- [x] Error messages (Levenshtein suggestions, internal var mapping)
- [x] Inline tests: `(test "name" expr expected)`
- [x] REPL with wasmtime mock NEAR runtime
- [x] Live testnet: `:push`, `:call`, `:call!` in REPL
- [x] Project system: `near.json` + `init`, `build`, `deploy`, `test`
- [x] Module imports: `(module name "path")` — text-level #include
- [x] Circular dependency detection
- [x] Tree-shaking: unused functions stripped from binary
- [x] Gas estimation per export
- [x] Solidity translator (experimental)
- [x] OutLayer WASI targets (P1 + P2 component model)
- [x] Web playground (browser WASM compiler)

---

## ❌ Gaps — Not Yet Implemented

### Borsh Serialization (partial)
- [ ] **Serialize F64** — `borsh-serialize: F64 not yet supported`
- [ ] **Serialize Enum** — `borsh-serialize: Enum not yet supported`
- [ ] **Deserialize F64** — `borsh-deserialize: F64 not yet supported`
- [ ] **Deserialize Vec of variable-length elements** — e.g. `Vec<String>`, `Vec<Vec<u8>>`
- [ ] **Deserialize variable-length field in nested struct**
- **Priority:** MEDIUM — most NEAR contracts use Borsh for state; these gaps limit which schemas can round-trip
- **Files:** `src/wasm_emit/borsh.rs` (lines ~240, ~419, ~489, ~556, ~601, ~767)

### Runtime Closures
- [ ] Lambdas only inlined at compile time — no runtime closure capture
- [ ] Can't pass a lambda that captures a runtime value to `map`/`filter` at runtime
- [ ] Workaround: `hof/map` etc. work because they inline the lambda body
- **Priority:** MEDIUM — limits metaprogramming but doesn't block real contracts
- **Files:** `src/wasm_emit/lambda.rs`

### Direct Recursion in WASM
- [ ] `(define (f) (... (f)))` does NOT emit a self-call
- [ ] Workaround: Y-combinator pattern `(f f args)` works, and `defn` tail-recursion compiles to loops
- [ ] Non-tail recursion requires manual transformation
- **Priority:** LOW-MEDIUM — most contract patterns are tail-recursive or iterative
- **Files:** `src/wasm_emit/call.rs` (self-passing call section)

### I32Load8/I32Store8 on NEAR
- [ ] **Broken on NEAR runtime** — return/store zeros
- [x] Workaround: masked I64Load/I64Store word operations (implemented as `emit_safe_store8`)
- **Impact:** Byte-level memory ops work but are slower (read-modify-write per byte)
- **Priority:** LOW — workaround is functional, just not optimal
- **Files:** `src/wasm_emit/mod.rs:424`

### F64 / Float Type
- [ ] No `f64` WASM type — everything is tagged i64
- [x] FP64 fixed-point (Q64.64) works for math-heavy paths
- **Impact:** Can't directly use WASM float ops or represent fractional values natively
- **Priority:** LOW — FP64 covers DeFi use cases adequately

### FT/NFT Standard Library
- [ ] No ergonomic wrappers for NEP-141 (FT), NEP-171 (NFT), NEP-145 (storage management)
- [ ] Can build them with raw promises + JSON, but verbose
- **Priority:** MEDIUM — would significantly improve contract ergonomics
- **Impact:** First-time users need ~50 lines for a basic FT transfer

### Borsh Schema Declarations
- [ ] No compile-time schema validation against actual struct usage
- [ ] Schemas are declarative but not enforced beyond serialization

---

## Known Bugs

- [ ] **REPL `:call!` stale value** — After mutable call, `:call` shows stale value (block cache). Wait a block and retry. Minor.
- [x] ~~**Double value_return**~~ — **FIXED.** `RETURN_FLAG` global prevents export wrapper's `value_return` when `near/return` was called explicitly.
- [x] ~~**Combined logging**~~ — **FIXED.** `(near/log "str" num)` uses two separate `log_utf8` calls.

---

## Architecture

```
input.lisp
    ↓ resolve_modules() — text-level #include
    ↓ parse → LispVal AST
    ↓ clojure desugar (defun, let, loop/recur → canonical forms)
    ↓ typecheck (catches type errors)
    ↓ compile_near() / compile_outlayer() / compile_outlayer_p2()
    ↓ tree_shake() — remove unused functions
    ↓ wasm-encoder Module (binary, no WAT strings)
    ↓ wasmparser validation
    ↓ .finish("_run")
output.wasm → deploy to NEAR / run via OutLayer
```

Memory layout:
- `48` — HANDLE_COUNT_ADDR (8 bytes)
- `56` — RUNTIME_HEAP_PTR (8 bytes, bump allocator)
- `64` — TEMP_MEM (return values, 8 bytes)
- `256` — AMOUNT_MEM (u128 deposit buffer, 16 bytes)
- `4096` — STDOUT_BUF (WASI output)
- `8192` — STORAGE_BUF (8 bytes, i64 storage temp)
- `8208` — STORAGE_U128_BUF (16 bytes, u128 storage temp)
- `8224` — KEY_BUF (256 bytes, composite key building)
- `16384` — INPUT_BUF (16KB for input JSON args)
- `32768` — RETURN_BUF
- `36864` — BORSH_BUF (4KB scratch)
- `49152` — HANDLE_TABLE_BASE (256 entries × 16 bytes)
- `200000` — HEAP_START (runtime bump allocator)

Tagged value scheme (3-bit tag in bottom bits):
- `0` Num — payload = 61-bit signed integer
- `1` Bool — payload = 0 (false) or 1 (true)
- `2` FnRef — payload = function index
- `3` Closure — payload = heap pointer
- `4` Nil — falsy sentinel
- `5` Str — payload = (heap_off | (len << 32))
- `6` Array — payload = heap pointer; heap layout: [count, elem0, elem1, ...]
- Falsy set: { Bool(false)=1, Nil=4 }. Num(0) is truthy (Lisp semantics).

## 2026-08-25 — found by tests/compiler-torture (first run)

### T4 (CRITICAL): closures over let vars share state across instances
- ✅ FIXED 2026-08-26 (round-3 fix 2): per-frame capture-cell table in
  run_compiled_lambda_inner — see round-3 section below. Two factory
  instances now get independent cells; same-invocation siblings share.

### T6: str-cat missing from interpreter (surface divergence, 3rd of its class)
- RESOLVED for the interpreter by dd0285d (corpus #1): str-cat now exists in
  eval_builtin with STRINGS-ONLY semantics (matches wasm_emit call_string.rs;
  see t6 header). The class itself (interpreter vs emitter surface drift)
  remains open — round 2 found more instances (near/has_key, near/kv-get,
  isqrt, wrap-add above).
- `(str-cat "x" 42)` → unknown builtin in lisp-run; exists in the WASM
  emitter path (GAPS str-cat key-collision entry references it). Same class
  as while/dotimes divergence. Note: hard-error change is what surfaced it
  (pre-fix it would have silently returned the form as data).

## 2026-08-25 — found by tests/compiler-torture round 2 (t10–t20)

### FIXED during round 2 (see git log for full messages)
- **max_call_depth guard was unreachable** (t11): dynamic-dispatch recursion
  consumed ~60KB native stack per frame; the 8MB main-thread stack aborted
  the process (SIGABRT, exit 134) around depth ~130 — before the 256 guard
  could fire. lisp-run now runs the VM on a 512MB-stack thread (13227af);
  "call depth exceeded" is reachable and clean (t11b pins the boundary:
  254 OK / 255 errors).
- **Cross-form forward references / mutual recursion** (t11): lisp-run
  compiled forms one at a time, so `(define (my-even? ...) (my-odd? ...))`
  above its partner was a hard compile error even though the library API
  supports it. CLI now pre-seeds define names (4196858).
- **abs(i64::MIN) panicked the process** (t18): exit 101 "attempt to negate
  with overflow". Both abs sites now use checked_abs → clean
  "integer overflow in abs" (9b98ae3).
- **(to-float "3.5") silently returned 0.0** (t12): the eval_builtin inline
  table had no Str arm; now parses like the (shadowed) dispatch impl
  (e35c7c9).
- **Dispatch-module errors were misreported as "unknown builtin"** (t13):
  e.g. out-of-range str-substring said "unknown builtin 'str-substring'"
  instead of "indices out of range". Errors now propagate (f79d26c).

### KNOWN — pinned, not fixed (see t-file markers)
- **User-fn arity: no validation (ARITY-PIN, t20).** `(f2 1)` with
  (define (f2 a b) ...) runs with b = nil (arith-coerced to 0); `(f2 1 2 3)`
  silently drops the extra arg. Should be a hard error. Fix needs agreement
  between the inlining compiler path and vm_call_lambda — not a one-liner.
- **Compiled arithmetic coerces non-numbers to 0** — ✅ FIXED (round-3
  fix 4, 2026-08-26): bare arith/comparisons now hard-error on non-numeric
  operands; see round-3 section below. String numerics via u128/*.
- **Division-by-zero message inconsistency** (t10): ✅ RESOLVED 2026-08-27 —
  one canonical pair everywhere: "division by zero" / "modulo by zero".
  Fixed sites: TypedBinOp I64 Div/Mod + SlotDivImm + builtin "mod" +
  tree-walker `/` ("div by zero" → "division by zero"). Bonus kills found in
  the sweep: (a) TypedBinOp U64 Div/Mod used `wrapping_div/rem` — a zero
  divisor PANICKED the process (exit 101); now clean Err (wasm traps natively
  via I64DivU, so interp-Err ≡ wasm-trap holds). (b) tree-walker `mod` used
  `i64::rem_euclid` via do_arith — zero divisor panicked; now guarded (every
  divisor in the fold). Stale pin updated: core_language test_mod_zero_divisor.
- **Inline builtin table shadows the dispatch modules with weaker
  semantics** (t13): str-length counts BYTES ("héllo" → 6; dispatch impl
  counts chars), str-split does NOT filter empty parts ("" → (""); dispatch
  filters), to-int of an unparseable string → 0 (dispatch errors). The
  dispatch versions are dead code for these names. Whichever semantics is
  canonical, the two tables should agree.
- **Recursion depth semantics** (t11, pinned as actual): direct
  self-recursion compiles to iterative CallSelf frames — NO depth limit;
  the 1M-op execution budget is the only ceiling (sum-to 10000 = 50005000
  runs clean). Mutual recursion and value-dispatched calls DO cross
  run_compiled_lambda each hop and are capped at max_call_depth=256 total
  crossings (254-deep chain + body = 256 OK; 255 fails).
- **lisp-run surface gaps vs this tracker** (t18/t19): near/has_key,
  near/kv-get (write near/kv exists, read does not — asymmetric),
  isqrt, wrap-add: all compile-error "unknown function or special form"
  in the CLI despite being listed as implemented above (they exist only on
  other paths).

### Semantics pinned as ACTUAL (not bugs)
- Floats print via Rust `{}`: 5.0 → "5" (no trailing .0); 2.5 → "2.5".
- u128/from-i64 of a negative i64 yields the SIGNED decimal string
  ("-9223372036854775808") — sign carried in the string representation.
- nil and '() are distinct ((= nil '()) → false, (nil? '()) → false).
- mod is euclidean ((mod -7 3) → 2); / truncates toward zero ((/ -7 2) → -3);
  u128/div truncates like python3 //.
- try/catch exists and catches hard errors; the caught value is a string
  that embeds the stack trace.

## Round 3 — four silent-wrong-answer classes (2026-08-25, session status)

Approved by JP 20:13. Order 1 → 3 → 4 → 2. Session ended early (parent steer 22:19);
state per fix:

### 1. 0-truthy → 0 is FALSY — ✅ LANDED (in commit a27b0b2)
- `src/helpers.rs` `is_truthy`: `Num(0)` and `Float(0.0)` now falsy; `""` and `'()`
  stay truthy per decision; `nil`/`Bool(false)` falsy as before.
- t20 TRUTHINESS-PIN flipped and passing; full torture sweep (32 files) exits
  with designed codes; cargo test 125/11 baseline intact.
- NOTE: the change rode into the wasm-sibling's wip commit a27b0b2 (they
  committed src/helpers.rs + t20 while both were dirty in my tree). Content is
  correct and verified; just don't credit-blame the commit message.
- wasm path: ✅ RECONCILED (2026-08-26 audit) — emit_is_truthy checks val==0
  (false|nil|zero falsy), landed riding 85765a0. Verified: nm_zero_is_falsy
  executes real wasm (returns else-branch); live interpreter probe agrees
  (0/nil/false falsy, ""/() truthy, (not 0)=true). Float(0.0) can't diverge on
  wasm — float literals hard-error ("unsupported expression form: Float(0.0)")
  until a float tag exists; when it does, add Float(0.0) to the wasm falsy set.
  Stale comments fixed in src/wasm_emit/mod.rs tag-scheme header.

### 3. User-fn arity validation — ✅ LANDED 2026-08-26 (fix 3)
Single choke point: `run_compiled_lambda` (src/bytecode.rs) validates
argc vs `num_fixed_params` after trace_push — every path funnels through it
(vm_call_lambda, const-fold inlining, apply/map/filter/reduce, try/catch
thunks, CallSelf entry). Missing AND extra args hard-error:
"arity mismatch: fn expects N args, got M"; variadic (&rest) requires
"at least N". Const-fold inlining now defers wrong-arity const calls to
runtime instead of baking wrong answers into constants (its Err arm
returns false → ops emitted → runtime check fires).
- t20 ARITY-PIN flipped (try/catch assertions); t21-arity-and-types.lisp
  written (14 assertions: direct/anonymous/apply/variadic/HOF + 3
  ARITH-PINs for fix 4).
- Full sweep: 33 files, no panics, all *b-err* files still exit 1 with
  designed messages. cargo test 125/11 baseline intact.
- wasm parity: compile-time via typing::type_check_program (when typecheck
  enabled) — semantics agree, wasm errors earlier.

### 4. Arith type errors — ✅ LANDED 2026-08-26 (fix 4)
Bare `+ - * / mod < <= > >=` are now i64/f64 ONLY. Non-numeric operands
hard-error: "type error: <op> expects numbers, got <a> <b>" (house style:
Display the actual values, like u128/* does). String numerics unchanged —
they go through u128/* (erc20 corpus unaffected, verified exit 0).
- num_arith (unchecked, dead code) deleted; num_arith_checked's coerce-to-0
  fallback replaced with the type error (both int and float coerce paths);
  num_cmp returns Result<bool,String> + op_name param — all 8 VM call sites
  (2 dispatch sites × 4 ops) rewritten with `?`.
- Const-fold path safe: it only folds pure fn CALLS (runtime arity/type
  checks now apply); peephole SlotAddImm/TypedBinOp fusions are i64/f64
  tag-guarded, so non-nums never reach them.
- t21 ARITH-PINs flipped + 5 new assertions (float-mix, cmp typing, u128
  unaffected). Sweep clean; cargo test 125/11 baseline intact.

### 2. T4 closure aliasing — ✅ LANDED 2026-08-26 (fix 2; round-3 COMPLETE)
Per-frame cell table in run_compiled_lambda_inner: PushClosure now
allocates/reuses capture cells from the CURRENT frame's table (main frame
+ each CallSelf frame gets a fresh one), not from the shared CompiledLambda
(cl.capture_cells) that aliased all invocations. Children still carry their
cells via the per-PushClosure inner_cloned.capture_cells (LoadCaptured/
StoreCaptured arms unchanged).
- Semantics now correct on all three axes: separate factory calls
  independent (1 2 1 2); siblings from ONE invocation share (inc/get probe:
  1 2 2); fresh pair → 0. Counter pattern persists per-closure.
- t20 T4-PIN flipped (1 2 1 2); t4-closures.lisp expanded with the
  sibling-sharing + fresh-invocation assertions (8 asserts).
- Full battery: sweep clean, cargo test 125/11 exact baseline.

Baseline at wrap: cargo test 125 passed / 11 failed (11 = sibling's known
wasi_emit outlayer/wasmtime failures).
[2026-08-27 post-clean rebuild: 150 passed / 10 failed — same wasi outlayer/
wasmtime family only; schnorr-wasm moved to workspace exclude so host
`cargo build/test --workspace` works from scratch (panic=abort is wasm-only).] Working tree left dirty ONLY with the
sibling's uncommitted src/wasm_emit/* changes — do not stash/revert those.

## Trace-Equivalence Harness (landed 2026-08-26)

`scripts/trace-equiv.py` — differential testing: every tests/equiv/*.lisp
probe (defines `(main)`) runs through BOTH surfaces:
- INTERP: lisp-run (probe + appended `(main)`), println lines
- WASM: near-compile → near-mock `--once _run {}`, `LOG:` lines
State isolated (state file wiped per run); 30s timeout → WASM_HANG class.
Categories: MATCH / DIVERGE / WASM_CERR / INTERP_ERR / BOTH_ERR / WASM_HANG.

First-run scoreboard: 13 probes → 8 MATCH, 5 WASM_CERR (surface gaps:
closures-as-values, lists/map, try/catch, deep equality), 0 DIVERGE.

### wasm bugs the harness caught (all fixed 2026-08-26, NEAR mode)
1. **to_str tag layout** — h_i64_to_str/h_to_str returned `len<<32|pos|5`
   (tag OR'd into payload, unshifted). Correct: `((len<<32)|pos)<<TAG_BITS|TAG_STR`.
   Symptom: every Num println silently logged nothing (garbage len/ptr →
   near-mock log_fn bounds check silently drops). 4 return sites fixed.
2. **h_i64_to_str magnitude** — `u = 0 - n` unconditionally → positives
   printed as 2^64-n (e.g. 42 → 18446744073709551574). Now `neg ? 0-n : n`.
3. **"false" constant** — 0x6573_6c61 ("alse") → 0x736c_6166; println false
   printed "alsee".
4. **NEAR str-cat local clobber** — fixed-name locals (`__sc_a`) → nested
   `(str-cat "x" (str-cat "y" "z"))` flattened to "yz". Depth-keyed
   (`__scn{d}_*`) like the P2/WASI arm. (3+ arg str-cat on NEAR is still
   binary-only — drops extras; TODO.)
5. **abs unsigned untag** — emit_untag is `shr_u`; abs(-5) = -5. Now
   signed `shr_s` inline in the abs arm.

### Documented surface gaps (WASM_CERR class — not bugs, scope)
- closures/lambda-as-values: "unknown function 'f'" (clean CERR).
  NOTE: top-level `(define c1 (mk))` referenced inside main reads as nil
  SILENTLY (no error, no-op) — should hard-error like local lambdas.
  TODO for wasm_emit.
- lists (car/cdr/map/len) emit broken code that fails wasm validation
  with a stack-underflow error at RUN time (should CERR at compile).
- try/catch, structural equality: unsupported (CERR).
- top-level program globals: interpreter has them; NEAR contract model
  compiles top-level defines as contract METHODS. Probes must use let
  locals (e07 pattern). PORTING HAZARD for corpus files.

### Round 4 — equivalence-driven wasm fixes (2026-08-26, PM)
1. **Value-define silent-nil** — bare Sym refs to top-level `(define i 7)`
   now CALL the synthesized 0-param fn (value_defines registry on the
   emitter; probe e14 MATCH). Was TAG_FNREF → println rendered nil.
2. **__h_arr_to_str helper (NEW)** — println on lists now renders
   "(e0 e1 ...)" per interpreter to_string: quoted strings, nested arrays
   (recursive), bools, zeros, separators. First hand-written wasm helper —
   bugs found & fixed en route: i64 consts fed to i32 stores (6), missing
   *8 in elem address, align-3 on 4-byte stores, copy j=1 skipping the
   nested '(' , quote w-advance off-by-one, plus cons/cdr missing
   I64Const after heap_bump (5 sites — stack underflow at validation).
3. **i64_to_str/h_to_str ZERO fast-path** — my round-3 PM fix OR'd 1<<32
   AFTER the dst<<3 shift (len field at bit 32, not 35) → len extracted 0
   → every zero rendered EMPTY ("(3 0 4)" → "(3  4)", println 0 → no LOG
   at all). Payload-first-then-shift now. (Morning verification missed it:
   4 printlns, only 3 LOGs — count your outputs.)
- Scoreboard: 16 probes — 11 MATCH / 5 WASM_CERR (e06 closures, e11/e12
  try-catch, e13 structural equality, + heterogeneous lists rejected by
  wasm typechecker: interp-only surface, document in porting guide) /
  0 DIVERGE. cargo test 125/11, torture sweep clean.

### Round 4b — more equivalence wins (2026-08-26 PM)
6. **Variadic arithmetic** — typing env was binary-only; interpreter left-folds
   (+ a b c ...). Added desugar in checker Call arm: + - * / min max fold
   to nested binary when >2 args; mod takes first two (extras dropped —
   interp semantics). Emitter was already n-ary (fold_binop).
7. **Structural equality (=)** — was raw tagged-i64 compare → dynamically
   built strings / fresh lists were never equal. NEW __h_val_eq helper
   (sig 1: (i64,i64)->bool): str = len+bytes loop, array = count+elementwise
   recursive, else raw. Typing: = special-cased reflexive (α→α→bool) —
   was num-only, rejected strings.
8. **!= on wasm — FIXED (was stale-binary artifact + non-structural)** —
   `(!= x y)` dispatch was never missing: call_core.rs:247 routes `"!="`
   → neq() (added in 0cb6d79), and NO parser desugar exists (lisp-run
   parse dump shows `Sym("!=")` survives verbatim). The repro's silent
   always-false came from a STALE target/debug binary: the round-4b WIP
   (val_eq helper) didn't compile (phantom FuncDef `sig` field), so cargo
   left the pre-0cb6d79 binary in place. Real fixes this round:
   (a) __h_val_eq made i64-returning (uniform (i64×n)→i64 sig — no sig
   field needed), returns I64Const, recursion via i64.eqz, local_count
   9→11 (locals 2..10, convention highest+1); (b) neq caller I32Eqz→
   I64Eqz; (c) typing: = and != special-cased reflexive α→α→bool (env
   scheme was num-only — rejected strings/lists outright). != now
   STRUCTURAL via __h_val_eq: strs, lists, dynamic strs all match interp.
- e21-str-equality / e22-structural-list-eq written; e17/e20 flip to MATCH
  once 8 is fixed; e16 (u128 try-based) & e18 (try/catch) stay CERR until
  wasm try exists. Heterogeneous lists: wasm typechecker rejects (documented).
- Post-fix scoreboard: 21 probes — 15 MATCH / 6 WASM_CERR (e06 e11 e12 e13
  e16 e18, all documented classes) / 0 DIVERGE. New probe e23-neq MATCH.
  cargo test 125 passed / 11 failed (sibling's known wasi_emit set).

## 2026-08-26 — ANCHOR DECISION: wasm is the semantic reference

**Decision (JP):** The wasm emission path (`src/wasm_emit/`) — what actually
deploys and runs on NEAR — is the semantic anchor. The Rust interpreter and
the differential-fuzz spec oracle must both be **trace-equivalent to wasm
semantics** (trap-style: no silent coercion, explicit errors), not to
Scheme/R7RS leniency.

Consequences applied today:
1. **Spec oracle hard-error alignment** (tests/test_differential_fuzz.rs):
   generic Add/Sub/Mul/Div/Mod and Lt/Le/Gt/Ge now mirror
   `num_arith_checked`/`num_cmp` exactly (U64×U64 wrapping/unsigned included)
   via `spec_arith_anchor`/`spec_cmp_anchor`. Killed 10 of 17 fuzz reds.
2. **Truthiness**: wasm `emit_is_truthy` treats Bool(false), Nil, and
   TAGGED==0 (Num 0) as falsy → spec oracle aligned to Rust's `is_truthy`
   (Num(0)/Float(0.0) falsy). Killed the JumpIfTrue divergence class.
   ⚠️ OPEN SUB-GAP: Rust treats **Float(0.0)** as falsy, but a *boxed*
   Float(0.0) would be truthy under the pure wasm tag check (only the raw
   tagged-0 pattern is falsy in wasm). Decide: special-case Float(0.0) in
   wasm emit, or accept Rust's rule as canonical. Low priority — Float
   conditionals are rare in generated code.
3. **Retired test**: `test_regression_empty_stack_pop_coercion` asserted the
   OLD lenient `nil+nil → 0`; now asserts the trap (both VMs agree).

**Result:** test_differential_fuzz 29/17 → **46/46**.

Remaining divergence classes (documented, unfixed):
- Fuzz mutation/long-program budget-boundary cases: Rust `eval_budget`
  (max_steps × 3) can exhaust before the spec VM's step loop on jump-heavy
  traces. Not a semantic divergence — an accounting mismatch between step
  units and budget units. Consider charging Rust 1 budget unit per op
  uniformly.

## 2026-08-27 — GAP SWEEP: t10 + near/storage_* family + promise_result

**Scope (JP picked menu item 3):** t10 zero-guards, near/storage_* p2 family
(kv asymmetry), string-storage builtin decision. Plus 2 bonus kills.

### t10 — ✅ RESOLVED (details in the t10 entry above)
Canonical pair "division by zero" / "modulo by zero" across all 6+ sites;
U64 wrapping-div panic and tree-walker rem_euclid panic killed.

### near/storage_* — ✅ LANDED: the string-safe storage family
The old state: interp ALIASED near/storage_set/get/has/remove to
near/store/load/remove wholesale (so `(near/storage_has k)` actually WROTE —
first-match arm bug), and the wasm emitter knew none of the names ("unknown
function"). New contract, both VMs:
- **near/storage_set / storage_write** (key:Str, val:Str) → Num(0). Non-Str
  key or value → hard error (interp message; wasm inline TAG_STR assert →
  Unreachable trap — same event class).
- **near/storage_get / storage_read** (key:Str) → Str; **"" on miss**.
- **near/storage_has / storage_has_key** (key:Str) → Num(1|0).
- **near/storage_remove** (key:Str) → Num(0).
- wasm storage_get bumps the heap (str-cat alloc idiom) and read_register
  copies the value bytes → returned Str is heap-stable; miss returns
  (TEMP_MEM, len 0) tagged Str.
- wasm returns bytes-in-bytes-out over raw host fns 17/18/19/20 — this is
  the HONEST mapping (host storage IS bytes). The typed-mode path shares
  call_near_storage via the try_domain chain; lambda.rs scan_host already
  declared the family.
- BUILTIN_NAMES (helpers.rs) + typing/checker.rs storage-schema tracker
  (aliases) extended. gap_c1 in test_type_system_gaps remains red (typed
  surface), unchanged from HEAD.

**⚠️ NEW DOCUMENTED SEAM — do not mix families per key:**
near/store writes the 8-byte TAGGED WORD (ptr|len for Str = heap garbage
across fresh-memory transactions — the erc20 hazard); near/storage_set
writes UTF-8 bytes. Both share the same on-chain key namespace. Reading a
near/store-written key via near/storage_get: interp hard-errors loudly
("non-string value ... mixing storage families"), wasm would decode the 8
binary bytes as a string. NEVER mix on one key. Corpus convention: strings
→ near/storage_*, Num/Bool/Nil → near/store|load.

### String-storage DECISION — resolved by the above
String values are legal ONLY through the near/storage_* family. erc20.lisp
(stores u128 decimal STRINGS via near/store — reloads garbage on-chain)
must MIGRATE to near/storage_set/get (NEXT; battery must stay 2/2).
Never-deploy note stands until migrated.

### near/promise_result — ✅ FIXED (2 bugs, found via near_cc_full_flow)
1. host_call(34) returns the promise STATUS; the emitter left it on the
   stack → invalid wasm ("values remaining on stack", func 6). Fixed: Drop.
2. The packed (len<<32|TEMP_MEM) result was pushed UNTAGGED → low 3 bits of
   an 8-aligned TEMP_MEM read as TAG_NUM → mistagged as a number. Interp
   returns LispVal::Str — now emit_tag_str() so both VMs agree.
   ⚠️ TEMP_MEM scratch exposure: like every register-to-buffer read, the
   returned Str points at shared scratch — copy (str-cat it) before any
   later host call if you need it to survive. Pre-existing convention.

### Stale pins flipped
- p2 bug_account_balance_high_unknown → near_account_balance_high
  (positive test; the builtin exists since the u64/schnorr work).
- core_language test_mod_zero_divisor → "modulo by zero" (t10 unification).

### Test results (my tree)
p2 87/87 · storage_family 5/5 (incl. fresh-memory persistence killer +
on-chain bytes shape) · money 16/16 · core 160/160 · regression 51/51 ·
safe 3/3 · differential fuzz 46/46 · u64 37/37.
Pre-existing reds on HEAD (verified via stash): lib wasi/outlayer 10
(infra/network), borsh_gaps 2, pure_types 1, schnorr 3, type_system_gaps 6,
u128_memory_bounds 19 + u128_safe_arithmetic 11 + wallet_diff 5
(near-compile PATH-not-found infra), wasm_fuzz 4 closure-family reds
(overnight marathon 56/4+2i). No regressions from the sweep.

### erc20 migration — ✅ LANDED 2026-08-27 (same day, next session block)
corpus/erc20.lisp + erc20-battery.lisp now on near/storage_set/get. load-str
normalize flipped: miss = Str("") → "0" via explicit str-length compare.
Bonus fix: the battery fuzz LCG (1103515245·s up to 2^61) tripped the 2^60
tagged-payload mul guard (money-safety range check, landed 8/26 — pre-existing
red, verified via stash). Rewritten as u128 string ops — SAME exact modular
sequence (fuzz final balances byte-match the python3 pins), FUZZ-OK n=200,
9/9 scripted blocks, zero errors. **erc20 is now deploy-safe.**

### Deploy-readiness pass 2026-08-27 (pre-deploy wasm smoke found 3 real bugs)
1. json_return_str/int ORDER BUG (wasm_emit/json.rs): prefix was written to
   INPUT_BUF before the arg expression evaluated — any arg containing
   json_get_* re-read input over the prefix (garbled {"x":"XYZ"}"XYZ"}).
   Fixed: arg expr first, then prefix. Proven via bisect minis A-I.
2. near-mock input() FIDELITY BUG: refused to overwrite a non-empty register;
   real NEAR always writes. predecessor+json_get in one contract left stale
   reg 0 -> parser walked "owner.test.near" hunting "amount" -> empty string
   -> u128 hard-trap. Fixed to always-write; ft_transfer un-trapped.
3. safe.lisp used variadic 3-arg str-cat (interpreter-only); wasm emitter is
   strict 2-ary. Nested via scripts/nest_strcat.py (assoc-preserving).
Deploy surface: corpus stays pristine; deploy/<name>/main.lisp GENERATED
(corpus + deploy/shims/<name>-shims.lisp) by scripts/gen_deploy.py. Full
lifecycle smokes green on wasm (erc20: mint/transfer/approve/transfer-from/
views/conservation; safe: init/propose/approve/refusals/views). Only untested
on-chain: safe 2-of-3 happy path (needs 2 distinct signers).

### NEXT
1. Deploy to kampy.testnet: safe.lisp (lisp5) and/or migrated erc20 (lisp5/6)
   — wasm's are built + smoke-proven (deploy/*/target/*.wasm).
2. The 4 closure fuzz reds (T4-adjacent).
