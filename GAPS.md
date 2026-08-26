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
- Two `(make-counter)` instances with the same binding name (`n`) share ONE
  cell: c1,c1,c2,c1 → 1,2,3,4 (expected 1,2,1,3). Param-capturing closures
  are independent (p1=101, p2=201); DIFFERENT binding names (n vs m) also
  independent. Mechanism: forward-captured let vars compile set!/read to
  StoreGlobal/LoadGlobal on the flat name-keyed env → env["n"] is global
  across all instances. Contract impact: two users' balances would alias.
  Fix requires per-instance closure cells (heap-allocated capture env at
  lambda creation) — architectural, not a patch. Until fixed: NEVER use
  same-named let-captured mutable state in two instances of a factory.

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
- **Compiled arithmetic coerces non-numbers to 0** (found via t14):
  `(+ "a" 1)` → 1, `(* (list 1 2) 10)` → 0 — silent wrong-answer class. The
  dispatch path (do_arith/as_num) errors properly; the hot compiled ops use
  num_val() which defaults to 0. Likely deliberate for the i64-only WASM tag
  scheme; flagging for a deliberate decision.
- **Division-by-zero message inconsistency** (t10): literal zero divisor
  const-folds to "integer overflow in div"; computed zero gives
  "division by zero". Same error, two messages.
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

### 4. Arith type errors — ❌ NOT STARTED (no code, no t21)
Next session: the lenient arms are `num_arith` (~src/bytecode.rs:4063),
`num_arith_checked` (~:4087, comment "Non-numeric: coerce to 0" at :4100),
and `num_cmp` (returns bool, non-numeric → false). num_arith/num_cmp don't
return Result — needs caller refactor to propagate
"type error: <op> expects numbers, got <type>". Decision recorded: bare
+ - * / mod < > <= >= are i64/f64 only; string numerics go through u128/*.

### 2. T4 closure aliasing — ❌ NOT STARTED (deliberately last; riskiest)
t20 T4-PIN and t4-closures.lisp still assert shared-cell (1,2,3,4) behavior.
Architectural: clone captured env cells at closure instantiation
(LoadCaptured/StoreCaptured in bytecode.rs). Run full battery after.

Baseline at wrap: cargo test 125 passed / 11 failed (11 = sibling's known
wasi_emit outlayer/wasmtime failures). Working tree left dirty ONLY with the
sibling's uncommitted src/wasm_emit/* changes — do not stash/revert those.
