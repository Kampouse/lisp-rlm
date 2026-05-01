# F* Verification Model vs Rust Implementation: Structured Diff

Generated: 2026-05-01
Scope: Op enum, LispVal variants, VM execution model, builtins

---

## 1. Op Variants in Rust but NOT in F*

These opcodes exist in `src/bytecode.rs` `Op` enum but have NO corresponding
variant in `verification/semantics/lisp/Lisp.Types.fst` `opcode` type.

| Rust Op | Purpose | Impact |
|---------|---------|--------|
| `PushSelf` | Push current function value for Y-combinator self-passing | F* cannot verify recursive self-application patterns |
| `TracePush(String)` | Push function name onto call trace (for stack traces) | Debugging/observability — F* proofs ignore tracing |
| `TracePop` | Pop function name from call trace | Same as above |
| `PushTry(usize)` | Push error handler (catch_pc) onto try_stack | F* has no exception model — try/catch is UNVERIFIED |
| `PopTry` | Remove current error handler | Same as above |

**Verdict**: 5 Rust opcodes are completely unverified. Most critically,
`PushTry`/`PopTry` implement structured exception handling that the F* model
simply skips.

---

## 2. Op Variants in F* but NOT in Rust

None. Every F* opcode has a Rust counterpart. The F* model is a strict subset.

---

## 3. Op Variants in Both but with Different Semantics

### 3a. ConstructTag — Parameter Types Differ

| Aspect | F* | Rust |
|--------|-----|------|
| Signature | `ConstructTag(string * nat * nat)` | `ConstructTag(String, u16, u8)` |
| 2nd param | `n_args` (number of items to pop) | `variant_id` (variant index) |
| 3rd param | `variant_idx` (variant index, UNUSED in body) | `n_fields` (number of items to pop) |
| Field names | Generated: `[("0", v0); ("1", v1); ...]` | Just `Vec<LispVal>` (positional) |

**Impact**: The F* model's Tagged values store named fields `list (string * lisp_val)`.
Rust's `Tagged` stores positional `Vec<LispVal>` with a `variant_id: u16`.
This means GetField in F* looks up by string key, while Rust looks up by integer
index. The semantics diverge for any code using deftype.

### 3b. TagTest — Variant Checking Scope

| Aspect | F* | Rust |
|--------|-----|------|
| Checks | `type_name` ONLY | Both `type_name` AND `variant_id` |

F* code (both Semantics and ClosureVM):
```
| Tagged (tn, _) -> tn = type_name   // ignores variant_idx
```

Rust code:
```
tn == type_name && *vid == *variant_id   // checks BOTH
```

**Impact**: F* proofs may pass for code where Rust would reject a TagTest
on the wrong variant of the same type. This is a SOUNDNESS GAP in the F*
model — it proves less than Rust actually enforces.

### 3c. SlotAddImm — Write-Back Divergence

| Aspect | F* Semantics (LispIR.Semantics) | F* ClosureVM | Rust |
|--------|-----|------|------|
| Slot writeback | YES (list_update) | NO | NO |
| Stack push | YES | YES | YES |

F* Semantics: `slots = slots'; stack = r :: s.stack`
F* ClosureVM: `stack = Num (n + imm) :: s.stack` (no slot update)
Rust: `stack.push(LispVal::Num(v + imm))` (no slot update)

**Impact**: The simpler F* model (LispIR.Semantics) writes back to the slot,
but the actual Rust VM and the more detailed F* ClosureVM do NOT. The
LispIR.Semantics proofs about SlotAddImm may not transfer. Also: the Rust
Op enum comment says "write back to slot AND push result" but the code does
not — documentation bug.

### 3d. OpDiv — Float-Awareness Gap

| Aspect | F* Semantics | F* ClosureVM | Rust |
|--------|-----|------|------|
| Float args | Float-aware (via num_arith) | Num/Num ONLY | Float-aware (via num_arith) |
| Div-by-zero | Returns Num 0 | Sets ok=false | Returns Err |

**Impact**: The F* ClosureVM model rejects float division and has different
error handling than Rust. F* Semantics agrees with Rust on float-awareness
but disagrees on div-by-zero behavior (silent 0 vs error).

### 3e. OpMod — Type Restriction

| Aspect | F* ClosureVM | Rust |
|--------|------|------|
| Float args | Rejected (ok=false) | Accepted (truncates via num_val) |
| Div-by-zero | ok=false | Returns Err |

### 3f. TypedBinOp — F64 Mode Simplification

| Aspect | F* ClosureVM | Rust |
|--------|------|------|
| F64 Add | `Num(a + b)` (returns Num!) | `Float(av + bv)` |
| F64 Sub | `Num(a - b)` (returns Num!) | `Float(av - bv)` |
| F64 others | `Num a` (identity) | Proper Float arithmetic |
| Input types | Only matches `Num, Num` | Matches `Float/Num` combinations |

**Impact**: F* ClosureVM's TypedBinOp F64 mode is WRONG — it returns Num
instead of Float, and only handles Num inputs. This is a known simplification
for Z3 tractability but means F* proofs about typed float ops are INVALID
with respect to the Rust runtime.

### 3g. RecurDirect — Fills Slots Differently

| Aspect | F* ClosureVM | Rust |
|--------|------|------|
| Slot fill | `fill_slots num_slots vals` (pads to num_slots) | `vec![Nil; cl.total_slots]` then fills params |
| Rest param | Not handled | Packed into rest_param_idx slot |

---

## 4. LispVal Variant Comparison

### 4a. F*-only Variants (NOT in Rust)

| F* Variant | Notes |
|-----------|-------|
| `Pair(lisp_val * lisp_val)` | Cons cell — Rust uses `List` for everything |

### 4b. Rust-only Variants (NOT in F*)

| Rust Variant | Notes |
|-------------|-------|
| `CaseLambda { cases, closed_env }` | Multi-arity dispatch — unverified |
| `Macro { params, rest_param, body, closed_env }` | Macro system — unverified |
| `Recur(Vec<LispVal>)` | Control-flow marker for loop/recur — unverified |
| `Memoized { func, cache }` | Memoization wrapper — unverified |

### 4c. Both but Structurally Different

| Variant | F* | Rust | Divergence |
|---------|-----|------|------------|
| Lambda | `(list string, lisp_val, list (string * lisp_val))` — 3-field tuple | 7-field struct: params, rest_param, body, closed_env, pure_type, compiled, memo_cache | F* has no rest_param, no compiled bytecode, no memo_cache |
| Dict/Map | `list (string * lisp_val)` — association list | `im::HashMap<String, LispVal>` — hash map | Same logical semantics, different performance. F* dict operations are O(n) linear scan. |
| Tagged | `string * list (string * lisp_val)` — (type_name, named fields) | `{ type_name: String, variant_id: u16, fields: Vec<LispVal> }` | F* has named fields, Rust has positional + variant_id. Field access semantics differ. |

---

## 5. VM Execution Model Comparison

### 5a. Architecture

| Aspect | F* | Rust |
|--------|-----|------|
| Models | 2: `LispIR.Semantics` (flat) + `LispIR.ClosureVM` (closure-aware) | 1: `run_compiled_lambda_inner` (unified) |
| State | Pure functional (lists, records) | Mutable (Vec, HashMap, Arc<RwLock<>>) |
| Error handling | `ok: bool` flag + `vm_result` | `Result<LispVal, String>` with descriptive errors |
| Termination | Fuel-based (eval_steps / closure_eval_steps) | Budget-based (eval_count + per-lambda ops counter) |
| Env model | Functional dict `list (string * lisp_val)` | `im::HashMap` + `Arc<RwLock<>>` shared state |

### 5b. F* Semantics vs F* ClosureVM Internal Divergence

The two F* models DISAGREE with each other:

| Aspect | LispIR.Semantics | LispIR.ClosureVM |
|--------|-------------------|-------------------|
| SlotAddImm writeback | YES | NO |
| LoadGlobal | No-op (advances PC) | Looks up in env dict |
| StoreGlobal | No-op (advances PC) | Updates env dict |
| StoreCaptured | No-op (advances PC) | Updates captured list |
| OpDiv div-by-zero | Returns Num 0 | Sets ok=false |
| PushFloat | Stores actual float | Replaced with `ff_of_int 0` |

This means proofs in LispIR.Semantics may not compose with proofs in
LispIR.ClosureVM.

### 5c. Features in Rust VM Not Modeled in F*

- **Try/catch exception handling** (PushTry/PopTry + try_stack)
- **Call tracing** (TracePush/TracePop + call_trace ring buffer)
- **Per-lambda execution budget** (ops counter + eval_budget)
- **Global shared environment** (Arc<RwLock<Env>> for nested calls)
- **Memoization** (memo_cache on Lambda + hash_args)
- **Constant folding** at compile time (try_const_fold)
- **Lambda inlining** (try_inline_call)
- **Peephole optimization** (fuse_ops pass)
- **Rest parameters** (rest_param_idx packing)
- **NEAR storage/context** (near_storage, near_context in EvalState)

---

## 6. Builtins Comparison

### 6a. Builtins Verified in F* (builtin_result in LispIR.ClosureVM)

Only 10 builtins have F* formalization:
`length`, `append`, `car`, `cdr`, `cons`, `list`, `str-concat`, `abs`, `min`, `max`

### 6b. Builtins in Rust (from helpers.rs BUILTIN_NAMES)

Over 200 builtins, including but not limited to:
- Arithmetic: +, -, *, /, mod, abs, min, max, floor, ceiling, round, sqrt, expt, pow, inc, dec
- Comparison: =, ==, !=, /=, <, >, <=, >=, equal?, eq?
- Collections: list, car, cdr, cons, len, append, nth, reverse, sort, range, zip, take, drop, last, butlast
- Higher-order: map, filter, reduce, find, some, every, for-each, fold-left, fold-right
- Strings: str-concat, str-contains, str-length, str-substring, str-split, str-trim, str-upcase, str-downcase, str-index-of, str-starts-with, str-ends-with, str=, str!=, str-join, str-chunk, str-replace + R7RS aliases
- Dict: dict, dict/get, dict/set, dict/has?, dict/keys, dict/vals, dict/remove, dict/merge, dict-ref, dict-set
- Predicates: nil?, list?, number?, string?, map?, bool?, zero?, positive?, negative?, even?, odd?, empty?, procedure?, symbol?, int?, float?, type?
- IO: print, println, read-file, write-file, load-file, file/read, file/write, file/exists?, file/list
- HTTP: http-get, http-post, http-get-json
- LLM: llm, llm-code, llm-batch
- NEAR: rlm/signature, rlm/format-prompt, rlm/trace, rlm/config
- RLM: rlm, sub-rlm, rlm-tokens, rlm-calls, read-all, load-file
- Type system: check, check!, matches?, valid-type?, type-of, defschema, validate, schema, infer-type, pure-type
- State: snapshot, rollback, rollback-to, save-state, load-state
- Shell: shell, shell-bg, shell-kill
- Conversion: to-float, to-int, to-num, to-string
- Runtime: now, elapsed, sleep, doc, pure, memoize
- Crypto: sha256, keccak256
- Deftype: tag-test, get-field
- + R7RS Scheme stdlib aliases

**Coverage**: F* verifies ~5% of Rust builtins (10 out of 200+).

### 6c. BuiltinCall Dispatch in VM

| Aspect | F* ClosureVM | Rust |
|--------|------|------|
| HOF support (map/filter/reduce/sort) | NOT modeled — calls builtin_result directly | Inline HOF handling with vm_call_lambda |
| Error propagation | Returns Nil on failure | Returns Err(String) |
| Lambda arguments to builtins | Not supported | Supported for map, filter, reduce, sort, for-each |

**Impact**: The F* model treats BuiltinCall as a pure function lookup.
In Rust, builtins like `map` and `filter` can accept lambda arguments and
invoke them, which is a side-effectful operation that the F* model does not
capture.

---

## 7. Summary of Verification Gaps

### Critical (proofs may be INVALID wrt Rust):

1. **TypedBinOp F64 mode** — F* returns Num, Rust returns Float. F* proofs
   about typed float arithmetic prove the WRONG thing.
2. **TagTest variant checking** — F* ignores variant_id, Rust checks it.
   Proofs may pass for programs Rust would reject.
3. **SlotAddImm writeback** — LispIR.Semantics writes back to slot,
   neither ClosureVM nor Rust do. Semantics proofs don't transfer.
4. **PushTry/PopTry exception handling** — Completely unverified.
   Any program using try/catch has zero formal guarantees.

### Moderate (incomplete coverage):

5. **190+ builtins unverified** — Only 10/200+ builtins have F* models.
6. **5 Op variants unverified** — PushSelf, TracePush, TracePop, PushTry, PopTry.
7. **4 LispVal variants unverified** — CaseLambda, Macro, Recur, Memoized.
8. **HOF builtins (map/filter/reduce)** — F* models them as pure lookups,
   Rust invokes lambdas with side effects.
9. **Rest parameters** — F* Lambda has no rest_param, Rust does.

### Low (documentation/modeling gaps):

10. **Dict representation** — Association list vs HashMap (same semantics).
11. **Tagged field representation** — Named vs positional (same semantics for correct programs).
12. **Op enum comment bug** — Rust SlotAddImm comment says "write back" but code doesn't.
13. **Two F* models diverge internally** — LispIR.Semantics vs LispIR.ClosureVM disagree on 6+ opcodes.
