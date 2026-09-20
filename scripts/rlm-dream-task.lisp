;; DREAM TASK — the model picks its own homework.
;; Reads the worst-Q digest, asks the model to author ONE task
;; targeting its own weakness, then PROVES the task before banking:
;;   ref probe    — reference solution must complete it (solvable)
;;   poison probe — wrong answer must fail the verifier (not vacuous)
;;   weakness link — task must cite a state from the digest (no farming)
;; Banked candidates are installed by scripts/rlm-taskgate.py (cap 2,
;; 48h expiry). Runs via nightly, not the task loop.
(load-file "rlm_runtime.lisp")
(load-file "scripts/rlm-tasks/policy.lisp")

(define (read-file-or f d)
  (try (read-file f) (catch e d)))

(define (llm-ask p)
  ;; one retry on transient shim failures ("model busy" during sweeps)
  (let ((r (try (llm p) (catch e (str-concat "LLMFAIL:" (to-string e))))))
    (if (str-contains r "LLMFAIL")
      (begin (sleep 8) (try (llm p) (catch e "LLMFAIL:dead")))
      r)))

(define WORST (read-file-or "data/rlm/dream/worst-states.txt"
  "argtype|fold-right|rc2|es2|i2 — Q: ladder -0.31"))

(define PROPOSAL
  (llm-ask (str-concat
    "You are the DREAM layer of a Lisp agent. The solver keeps FAILING in these states:\n\n"
    WORST "\n\n"
    "Your job: pick ONE failure state and write a SMALL practice task for it.\n"
    "The task must exercise the same skill that is failing (same builtin, same shape).\n"
    "It must be solvable in one or two tiny lisp expressions.\n\n"
    "Answer with EXACTLY these 6 tagged lines, nothing else:\n"
    "STATE: <copy the exact failing state string from above>\n"
    "TASK: td_<short_name_up_to_20_chars>\n"
    "PROMPT: <imperative task, 40-300 chars, NO double quotes, must ask to store the result via (rlm-set answer ...) and finish via (rlm-set Final true)>\n"
    "VERIFY: <lisp expression using only the variable a and numbers, true iff a is the correct answer>\n"
    "REF: <single line of lisp: begin form that sets the CORRECT answer via rlm-set answer then (rlm-set Final true)>\n"
    "POISON: <same shape as REF but with a WRONG answer>\n\n"
    "Example:\n"
    "STATE: argtype|fold-right|rc2|es2|i2\n"
    "TASK: td_foldsum\n"
    "PROMPT: Compute the sum of the list (list 2 3 5 7) using fold-right; store it via (rlm-set answer ...) then (rlm-set Final true).\n"
    "VERIFY: (= a 17)\n"
    "REF: (begin (rlm-set answer (fold-right (lambda (x acc) (+ x acc)) 0 (list 2 3 5 7))) (rlm-set Final true))\n"
    "POISON: (begin (rlm-set answer 999) (rlm-set Final true))")))

(define (line-of tag s)
  (let ((i (str-index-of s (str-concat tag ": "))))
    (if (< i 0) ""
      (let ((rest (str-substring s (+ i (+ (str-length tag) 2)) (str-length s))))
        (let ((j (str-index-of rest "\n")))
          (if (< j 0) rest (str-substring rest 0 j)))))))

(define STATE   (line-of "STATE"   PROPOSAL))
(define SLUG    (line-of "TASK"    PROPOSAL))
(define PROMPT  (line-of "PROMPT"  PROPOSAL))
(define VERIFY  (line-of "VERIFY"  PROPOSAL))
(define REF     (line-of "REF"     PROPOSAL))
(define POISON  (line-of "POISON"  PROPOSAL))

;; ---- static validation ----
(define (ok-slug s)
  (and (>= (str-length s) 4) (<= (str-length s) 24)
       (= (str-index-of s "td_") 0)
       (not (str-contains s "."))
       (not (str-contains s "/"))
       (not (str-contains s "\""))))

(define VALID
  (and (ok-slug SLUG)
       (>= (str-length PROMPT) 40)
       (<= (str-length PROMPT) 400)
       (not (str-contains PROMPT "\""))
       (> (str-length VERIFY) 4)
       (str-contains VERIFY " a ")
       (> (str-length REF) 10)
       (> (str-length POISON) 10)
       (not (= REF POISON))
       (not (str-contains REF "\""))
       (not (str-contains POISON "\""))
       ;; weakness link: the cited state must come from the digest
       (str-contains WORST STATE)))

;; ---- assemble the task body (single begin form; probes eval it) ----
(define BODY
  (str-concat
    "(begin\n"
    "  (rlm-set __trace_id \"" SLUG "\")\n"
    "  (define (task-verify a) " VERIFY ")\n"
    "  (run-rlm \"" PROMPT "\")\n"
    "  (if (task-verify (rlm-get answer)) nil (rlm-set Final nil))\n"
    "  (write-trace)\n"
    "  (println (str-concat \"RLMDUMP task=" SLUG "\"))\n"
    "  (println (str-concat \"RLMDUMP iterations=\" (to-string (rlm-get iteration))))\n"
    "  (println (str-concat \"RLMDUMP answer=\" (to-string (rlm-get answer))))\n"
    "  (println (str-concat \"RLMDUMP Final=\" (to-string (rlm-get Final))))\n"
    ")"))

;; ---- probes: run the body with stubbed llm-code ----
;; NOTE: eval-scoped defines CANNOT shadow the llm-code builtin — the
;; stub must be defined at file top-level and read its payload from
;; rlm_state (same mechanism the nightly task files rely on).
(rlm-set __stub "")
(define (llm-code ctx) (rlm-get __stub))
(define (probe-run stub-code)
  (begin
    (rlm-set __stub stub-code)
    (init-rlm "dream-probe")
    (eval (read BODY))
    (rlm-get Final)))

(define REF-OUT    (probe-run REF))
(define POISON-OUT (probe-run POISON))

(define PROBE-OK (and (= REF-OUT true) (not POISON-OUT)))

;; ---- full file for nightly install ----
(define TASKFILE
  (str-concat
    ";; DREAMED TASK — agent-authored, probe-verified (ref solves, poison fails)\n"
    ";; targets state: " STATE "\n"
    ";; dreamed " (to-string (now)) "\n"
    "(load-file \"rlm_runtime.lisp\")\n"
    "(load-file \"scripts/rlm-tasks/policy.lisp\")\n"
    "(load-file \"scripts/rlm-tasks/q-table.lisp\")\n"
    "(load-file \"scripts/rlm-tasks/custom-actions.lisp\")\n"
    "(rlm-set __policy POLICY_ID)\n"
    "(define (llm-code ctx)\n"
    "  (llm (str-concat GRAMMAR \"\\n\\nTASK CONTEXT:\\n\" ctx)))\n"
    BODY "\n"))

(if (and VALID PROBE-OK)
  (begin
    (write-file "candidates/task-proposal.lisp" TASKFILE)
    (println "DREAMTASK: banked candidate")
    (println (str-concat "DREAMTASK: slug=" SLUG " ref=completed poison=failed")))
  (println (str-concat
    "DREAMTASK: rejected — valid=" (to-string VALID)
    " ref=" (to-string REF-OUT) " poison=" (to-string POISON-OUT)
    " slug=" (str-substring SLUG 0 24))))
