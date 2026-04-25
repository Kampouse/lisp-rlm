# Perpetual Verified Agent Runtime — Bootstrap Plan

## Architecture

```
Layer 0: OS (macOS, Linux)
Layer 1: Rust kernel (compiled, immutable, never patched)
Layer 2: Harness (Lisp, patchable, gated)
Layer 3: Agent code (Lisp, freely patchable by LLM)
```

Same eval/verify/patch loop at every layer, different safety thresholds.

---

## Layer 1 — Rust Kernel (Immutable)

The frozen foundation. ~10K lines already exist.

**Responsibilities:**
- Parse Lisp into AST
- Assign node IDs and source spans
- CPS eval + bytecode VM
- Budget enforcement (eval count)
- Capability enforcement
- Trace logging
- Patch registration and verification
- Persistence (save/load state to disk)

**What the kernel NEVER does:**
- Self-modify
- Call the LLM directly
- Make policy decisions
- Skip verification

**Current status:** ✅ Done. `src/eval/`, `src/parser.rs`, `src/types.rs`, `src/bytecode.rs`

**What needs adding:**

| Feature | Files | Lines |
|---------|-------|-------|
| Node IDs + source spans | `parser.rs` | ~60 |
| `checked-fn` special form | `eval/mod.rs` | ~100 |
| `defpatch` + patch registry | new `src/patch.rs` | ~150 |
| Capability scanner | new `src/capability.rs` | ~80 |
| Disk persistence | new `src/persist.rs` | ~120 |
| `load-directory` builtin | `eval/mod.rs` | ~30 |

Total: ~540 new lines on existing 10K.

---

## Layer 2 — Harness (Lisp, Patchable, Gated)

The agent's operating system. Written in Lisp, evolves through patches, but stricter verification.

**File:** `harness.lisp`

```lisp
;; === Scheduler ===
;; The main loop. Wakes up, checks events, picks actions, executes.

(defpatch scheduler-run ()
  :capabilities (scheduler)
  (let ((events (inbox-drain))
        (intentions (load-intentions)))
    (if (or events intentions)
        (let ((action (choose-action events intentions)))
          (if (policy-check action)
              (execute action)
            (ask-human action)))
      (noop))))

;; === Intention Store ===
;; Standing intentions: things the agent keeps caring about.

(defvar *intentions* (list))

(defpatch register-intention (intent)
  :capabilities (memory)
  (validate-intention intent)
  (push intent *intentions*))

(defpatch load-intentions ()
  (filter (lambda (i) (eq (get i :status) 'active))
          *intentions*))

(defpatch archive-intention (intent)
  (set intent :status 'done)
  (save-state "state/intentions" *intentions*))

(defpatch intention-done? (intent)
  ;; Check if the done-when condition is met
  (eval-condition (get intent :done-when) intent))

;; === Policy Gate ===
;; Decides if an action is allowed without human approval.

(defpatch policy-check (action)
  :capabilities (policy)
  (and (budget-remaining?)
       (capability-allowed? (action-capability action))
       (not (requires-human? action))))

(defpatch requires-human? (action)
  (member (action-type action)
          '(shell network send-message spend deploy delete publish)))

;; === Tool Attention ===
;; Only load tools relevant to the current intention.

(defpatch score-tools (intention)
  (let ((tools (all-tools)))
    (take 5 (sort-by-relevance intention tools))))

;; === Budget ===
;; Track spending across the loop.

(defvar *budget* (map "daily-limit" 1000 "used" 0))

(defpatch budget-remaining? ()
  (< (get *budget* "used") (get *budget* "daily-limit")))

(defpatch budget-spend (amount)
  (set *budget* "used" (+ (get *budget* "used") amount))
  (save-state "state/budget" *budget*))

;; === Verification Levels ===
;; Different trust for different layers.

(defvar *patch-levels*
  '((harness . (:contract :capability :human-approve))
    (agent . (:syntax :capability))))

;; === Heartbeat ===
;; Called by the Rust kernel on schedule.

(defpatch heartbeat ()
  (scheduler-run)
  (checkpoint))

;; === Persistence ===
;; Save state so reboot is clean.

(defpatch checkpoint ()
  :capabilities (file-write)
  (save-state "state/intentions" *intentions*)
  (save-state "state/memory" *memory*)
  (save-state "state/budget" *budget*)
  (save-patches "patches/" *accepted-patches*)
  (trace-write '(checkpoint)))

;; === Boot ===
;; Called by Rust kernel on startup.

(defpatch boot ()
  (load-directory "patches/")
  (restore-state "state/")
  (trace-write '(booted))
  (scheduler-run))
```

---

## Layer 3 — Agent Code (Lisp, Freely Patchable)

Task-specific code, learned patterns, strategies. The LLM writes these.

**Directory:** `patches/`

```lisp
;; patches/001-example.lisp
(defpatch greet-user (name)
  (str-concat "Hello, " name))
```

**Patch pipeline:**

```
LLM proposes patch
  → parse to AST
  → validate syntax
  → check capabilities
  → check contracts (if checked-fn)
  → run tests
  → if layer 2: require human approval
  → if layer 3: auto-accept
  → write to patches/ directory
  → eval into runtime
```

---

## The Intent Loop

The perpetual loop that drives the agent:

```
wake up
  → what do I care about?        (intentions)
  → what happened since I slept?  (events)
  → what should I do?             (choose action)
  → am I allowed?                 (policy gate)
  → do it or ask human            (execute)
  → remember what happened        (trace)
  → save state                    (checkpoint)
  → sleep
```

**No LLM call for routine stuff.** Intentions encode patterns:

```lisp
(intention
  :id "watch-pr-42"
  :goal "get PR 42 merged"
  :status active
  :trigger (or timer ci-failed review-comment)
  :next-action check-ci-status
  :done-when merged
  :budget (max 10-per-day))
```

The loop:
1. Find active intentions whose trigger fired
2. Run their `next-action`
3. Update based on result
4. Check if done → archive
5. LLM only involved for novel situations

---

## Disk Layout

```
lisp-rlm/
├── Cargo.toml
├── src/                        ← Rust kernel (frozen)
│   ├── eval/
│   ├── patch.rs                ← NEW: patch registry
│   ├── capability.rs           ← NEW: capability scanner
│   ├── persist.rs              ← NEW: disk save/load
│   └── ...
├── runtime/                    ← Lisp runtime (evolves)
│   ├── harness.lisp            ← Layer 2: agent OS
│   ├── patches/                ← Layer 3: accepted patches
│   │   ├── 001-*.lisp
│   │   └── 002-*.lisp
│   ├── state/                  ← persisted state
│   │   ├── intentions.json
│   │   ├── memory.json
│   │   └── budget.json
│   └── trace/                  ← audit log
│       └── 2026-04-25.log
└── BOOTSTRAP.md                ← this file
```

**Boot sequence:**
1. Rust kernel starts
2. Load `runtime/harness.lisp`
3. Load all files in `runtime/patches/` in order
4. Restore state from `runtime/state/`
5. Call `(boot)` → starts scheduler loop

**Reboot = re-eval everything from disk.** Memory is just files.

---

## Verification Levels

Practical first, formal later.

| Level | What | When |
|-------|------|------|
| 1 | Syntax valid | Every patch |
| 2 | Capability check | Every patch |
| 3 | Runtime contracts (`checked-fn`) | If annotated |
| 4 | Tests/assertions | If provided |
| 5 | Symbolic simplification | Later |
| 6 | SMT checks | Much later |
| 7 | Proof-carrying code | Research |

---

## Safety Posture

```
LLM proposes.
Verifier disposes.
Runtime records.
Human governs.
```

**Defaults: deny everything.**

| Capability | Default |
|------------|---------|
| shell | deny |
| network | deny |
| file-write | deny |
| messaging | deny |
| spend | deny |
| deploy | deny |
| delete | deny |
| publish | deny |
| file-read | allow |
| memory | allow |
| scheduler | allow |

**Sharp actions always require human approval.**

---

## MVP Build Order

### Week 1: Foundation
- [ ] Node IDs + source spans in parser
- [ ] `checked-fn` special form (runtime contracts)
- [ ] `defpatch` + patch registry
- [ ] Capability scanner (AST walk for dangerous symbols)
- [ ] Disk persistence (save/load state)

### Week 2: Harness
- [ ] Write `harness.lisp`
- [ ] Scheduler + heartbeat loop
- [ ] Intention store
- [ ] Policy gate
- [ ] Boot sequence (load harness → patches → state → run)

### Week 3: Verification
- [ ] Patch verification pipeline (syntax → caps → contracts → tests)
- [ ] Verification scoring (confidence per patch)
- [ ] Trace logging
- [ ] Human approval interface

### Week 4: Intelligence
- [ ] Tool attention (score relevance, load top-k)
- [ ] LLM integration in the loop (novel situations only)
- [ ] Intention lifecycle (create → active → done → archive)
- [ ] Memory as patches (learned patterns survive reboots)

### Month 2: Type System
- [ ] HM type inference for simple functions
- [ ] ADT constructors + pattern matching
- [ ] Remove universal nil (Unit + Option + Result)
- [ ] Effect types

### Month 3: Formal
- [ ] Symbolic simplifier
- [ ] SMT integration
- [ ] Proof obligations
- [ ] Verification score dashboard

---

## Guiding Principle

Build a tiny Lisp machine where the LLM can dream, but every dream must:
1. Become typed AST
2. Pass the verifier
3. Fit the budget
4. Hold the right capability
5. Leave a trace

The kernel is physics. Patches are evolution. The verifier is the immune system. The LLM is imagination.
