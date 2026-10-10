# Perpetual Verified Agent Runtime — Bootstrap Plan

## Architecture

```
Layer 0: OS (macOS, Linux)
Layer 1: Rust kernel (compiled, immutable, never patched)
Layer 2: Harness (Lisp, patchable, gated)
Layer 3: Agent code (Lisp, freely patchable by LLM)
```

Same eval/verify/patch loop at every layer, different safety thresholds.

The LLM is a builtin inside the runtime, not the driver. The runtime calls it. It proposes. The kernel decides.

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
- Clock: expose `(now)` and `(elapsed since)` builtins

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
| `(now)` / `(elapsed)` builtins | `eval/mod.rs` | ~20 |

Total: ~560 new lines on existing 10K.

---

## Layer 2 — Harness (Lisp, Patchable, Gated)

The agent's operating system. Written in Lisp, evolves through patches, but stricter verification.

**File:** `harness.lisp`

```lisp
;; === Clock ===
;; Kernel exposes (now) → unix timestamp. Everything else is subtraction.

(defpatch time-since (timestamp)
  (- (now) timestamp))

;; === Scheduler ===
;; Wakes up, ranks intentions, executes sequentially within budget.

(defpatch scheduler-run ()
  :capabilities (scheduler)
  (let ((candidates (rank-intentions (load-intentions) (inbox-drain) *budget*)))
    ;; Execute as many as budget allows, one at a time (no parallel by default)
    (loop for action in candidates
          while (budget-remaining?)
          do (run-action action))))

(defpatch run-action (action)
  (if (policy-check action)
      (let ((result (execute action)))
        (handle-result (action-intention action) result)
        (trace-write action result)
        (checkpoint))
    (ask-human action)))

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
  (eval-condition (get intent :done-when) intent))

;; === Intention Types ===
;; Intentions declare their lifecycle upfront.

;; perpetual  — never done, runs forever (monitoring, health checks)
;; completable — has a finish line (merge PR, fix bug)
;; one-shot — do once, then archive (deploy hotfix)
;; recurring — runs on schedule, each run independent (daily backup)

(defpatch handle-result (intention result)
  (case (get intention :type)
    ;; Never archive, just update last-acted
    (perpetual
      (set intention :last-acted (now)))

    ;; Check done-when condition
    (completable
      (if (eval-condition (get intention :done-when))
          (archive-intention intention)
        (set intention :last-acted (now))))

    ;; Archive after first run
    (one-shot
      (archive-intention intention))

    ;; Track last run, reset for next trigger
    (recurring
      (set intention :last-run (now))
      (set intention :status 'waiting))))

;; === Priority Scoring ===
;; Three inputs, one score: urgency × relevance × cost-efficiency

(defpatch urgency (intention events)
  (let ((deadline (get intention :deadline))
        (trigger (get intention :trigger)))
    (cond
      ;; Hard deadline within 1 hour
      ((and deadline (< (time-until deadline) 3600))
       1.0)
      ;; Trigger just fired
      ((trigger-matched? trigger events)
       0.8)
      ;; Trigger close to firing
      ((trigger-close? trigger events)
       0.5)
      ;; No trigger, just timer
      (t 0.2))))

(defpatch relevance (intention events)
  (let ((tags (get intention :tags))
        (event-tags (map event-tags events)))
    (/ (count-intersection tags event-tags)
       (max (length tags) 1))))

(defpatch cost-efficiency (intention budget)
  (let ((cost (estimate-cost (get intention :next-action))))
    (cond
      ((= cost 0) 1.0)                                    ;; free action
      ((< cost (* 0.01 (get budget :daily-limit))) 0.9)   ;; cheap
      ((< cost (* 0.10 (get budget :daily-limit))) 0.6)   ;; moderate
      (t 0.3))))                                          ;; expensive

(defpatch score-intention (intention events budget)
  (let ((u (urgency intention events))
        (r (relevance intention events))
        (e (cost-efficiency intention budget))
        (score (+ (* 0.5 u) (* 0.3 r) (* 0.2 e))))
    (set intention :score score)
    score))

(defpatch rank-intentions (intentions events budget)
  (sort (lambda (a b) (> (get a :score) (get b :score)))
        (map (lambda (i) (score-intention i events budget))
             intentions)))

;; === Starvation Prevention ===
;; Nothing waits forever. If an intention hasn't been acted on
;; beyond its max-wait, urgency auto-bumps to 0.9.

(defvar *check-history* (map))

(defpatch find-starved (intentions)
  (filter (lambda (i)
            (let ((last-check (get *check-history* (get i :id))))
              (or (nil? last-check)
                  (> (time-since last-check)
                     (get i :max-wait (* 24 3600))))))
          intentions))

(defpatch balance-intentions (ranked budget)
  (let ((starved (find-starved ranked)))
    (when starved
      (set (car starved) :score 0.9)))  ;; force into contention
  ranked)

;; === Conflict Resolution ===
;; Two layers: declared conflicts and resource conflicts.

;; Layer 1: intentions declare what they conflict with
;;
;; (intention :id "trade-near"
;;   :conflicts-with (preserve-capital)
;;   :priority 2)
;;
;; (intention :id "preserve-capital"
;;   :priority 1)  ;; lower number = higher priority

;; Layer 2: runtime detects resource conflicts (budget, wallet)
(defpatch find-resource-conflicts (actions)
  (let ((spends (filter spends-budget? actions)))
    (if (> (length spends) 1)
        (let ((total (reduce + (map action-cost spends))))
          (if (> total (budget-remaining))
              (list :conflict :over-budget spends)
            nil))
      nil)))

(defpatch resolve-conflicts (candidates)
  (let ((declared (filter-declared-conflicts candidates))
        (resource (find-resource-conflicts candidates)))
    ;; Declared: lower priority number wins
    ;; Resource: reject if over budget
    ;; Tied: ask human
    (remove-resolved-conflicts candidates declared resource)))

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
;; Avoids tool-schema bloat, reduces token cost.

(defpatch score-tools (intention)
  (let ((tools (all-tools)))
    (take 5 (sort-by-relevance intention tools))))

;; === Budget ===

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
;; Save state so reboot is clean. Every accepted patch is a file on disk.

(defpatch checkpoint ()
  :capabilities (file-write)
  (save-state "state/intentions" *intentions*)
  (save-state "state/memory" *memory*)
  (save-state "state/budget" *budget*)
  (save-state "state/check-history" *check-history*)
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

**The LLM is invoked as a builtin, not as the driver:**

```lisp
;; Inside the intent loop, only for novel situations:
(defpatch handle-novel (situation)
  (let ((proposal (ask-llm (format-prompt situation))))
    (let ((ast (parse proposal)))
      (if (verify ast)
          (if (requires-human? ast)
              (ask-human ast)
            (eval ast))
        (trace-write '(rejected proposal))))))
```

The LLM never runs anything. It proposes. The eval loop runs. The LLM is a read-only oracle — it generates text, the kernel decides what to do with it.

---

## The Intent Loop

The perpetual loop that drives the agent:

```
wake up
  → what do I care about?        (intentions)
  → what happened since I slept?  (events)
  → score and rank intentions     (urgency × relevance × cost)
  → detect conflicts              (declared + resource)
  → pick top actions within budget
  → am I allowed?                 (policy gate)
  → do it or ask human            (execute sequentially)
  → update intention state        (perpetual/completable/one-shot/recurring)
  → remember what happened        (trace)
  → save state                    (checkpoint)
  → sleep
```

**No LLM call for routine stuff.** Intentions encode patterns.

**Execution is sequential, not parallel.** Actions run one at a time because they share resources (budget, wallet). Within one intention, `progn` sequences sub-actions. Parallel execution is a capability that requires human approval.

---

## Intention Types

Intentions declare their lifecycle upfront:

### Perpetual — never done, runs forever
```lisp
(intention
  :id "watch-prices"
  :type perpetual
  :trigger (timer :every 1800)  ;; every 30 minutes
  :next-action check-prices
  :max-wait 7200)               ;; starve alert if not checked in 2 hours
```

### Completable — has a finish line
```lisp
(intention
  :id "get-pr-42-merged"
  :type completable
  :trigger (or ci-failed review-comment timer)
  :next-action check-ci-status
  :done-when (pr-state "42" 'merged)
  :budget (max 10-per-day)
  :max-wait 14400)
```

### One-shot — do once, archive
```lisp
(intention
  :id "deploy-hotfix"
  :type one-shot
  :next-action (progn (build) (deploy) (verify))
  :priority 1)
```

### Recurring — runs on schedule, each run independent
```lisp
(intention
  :id "daily-backup"
  :type recurring
  :trigger (timer :cron "0 3 * * *")
  :next-action backup-state)
```

---

## Priority Scoring

Three factors, weighted:

| Factor | Weight | Purpose |
|--------|--------|---------|
| Urgency | 50% | Act on what's time-sensitive |
| Relevance | 30% | Act on what matches current events |
| Cost-efficiency | 20% | Prefer cheap actions, dampen expensive ones |
| Starvation guard | override | Nothing waits forever |

**Starvation prevention:** if an intention hasn't been acted on beyond its `:max-wait` (default 24 hours), urgency auto-bumps to 0.9 regardless of other scores. The agent never forgets.

---

## Conflict Resolution

Two layers:

**1. Declared conflicts** — intentions say what they conflict with, lower priority number wins:
```lisp
(intention :id "trade-near" :conflicts-with (preserve-capital) :priority 2)
(intention :id "preserve-capital" :priority 1)  ;; wins
```

**2. Resource conflicts** — runtime detects when two actions would consume the same budget/wallet. Reject if over budget.

**3. Tied** — ask human.

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
│   │   ├── budget.json
│   │   └── check-history.json
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
| parallel | deny |
| file-read | allow |
| memory | allow |
| scheduler | allow |
| llm | allow |

**Sharp actions always require human approval.**

---

## Why This Design

**The LLM is the most expensive part.** It should be the last resort, not the first. Most of what an agent does is repetitive: check status, compare against threshold, update state, send notification. That's just `if` statements.

The intent loop is a cache for LLM decisions. Last time you saw this pattern, the LLM decided X. Now that decision is encoded as an intention running as Lisp. Next time, skip the LLM entirely.

**Sequential execution** prevents resource conflicts by default. Parallel is a capability, not the norm.

**The LLM is inside the runtime, not wrapping it.** The runtime calls `(llm "prompt")` like any other function. No special privileges. Same capability gate, same budget check, same verification.

---

## MVP Build Order

### Week 1: Foundation
- [ ] Node IDs + source spans in parser
- [ ] `(now)` / `(elapsed)` builtins
- [ ] `checked-fn` special form (runtime contracts)
- [ ] `defpatch` + patch registry
- [ ] Capability scanner (AST walk for dangerous symbols)
- [ ] Disk persistence (save/load state)

### Week 2: Harness
- [ ] Write `harness.lisp`
- [ ] Scheduler + heartbeat loop
- [ ] Intention store with types (perpetual/completable/one-shot/recurring)
- [ ] Priority scoring (urgency × relevance × cost-efficiency)
- [ ] Starvation prevention
- [ ] Conflict resolution (declared + resource)
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

The goal: set an intention, walk away, and it just runs. For days. For weeks. You only hear from it when something actually needs your attention. That's not babysitting — that's delegation.
