;; DREAM TASK — propose a new escalation action (agent-written harness).
;; Reads the worst failure states (from rlm-dream.py digest), asks the
;; model to write ONE new action as lisp, self-probes it, and banks the
;; candidate file for the A/B gate. Runs via nightly, not the task loop.
(load-file "rlm_runtime.lisp")
(load-file "scripts/rlm-tasks/policy.lisp")
(load-file "scripts/rlm-tasks/q-table.lisp")

(define (read-file-or f d)
  (try (read-file f) (catch e d)))

(define WORST (read-file-or "data/rlm/dream/worst-states.txt"
  "argtype|fold-right|rc2|es2|i2 — Q: ladder -0.31
runtime|reverse|rc3|es3|i3 — Q: ladder -0.28"))

(define PROPOSAL
  (llm (str-concat
    "You are the DREAM layer of a Lisp agent. The solver keeps FAILING in these states:\n\n"
    WORST "\n\n"
    "The current escalation menu is: doc, hint, ban, temp08, temp10, budget2, stop, ladder.\n"
    "NONE of these is working for the states above. INVENT one new action.\n\n"
    "Write lisp that registers it. EXACTLY this shape:\n"
    "(rlm-register-action \"myaction\" (lambda (rc)\n"
    "  (str-concat \"\\nSOME helpful escalation text.\\n\")))\n\n"
    "Rules: the lambda takes rc (repeat count number) and MUST return a STRING.\n"
    "Allowed: str-concat, to-string, if, let, rlm-get (keys: __last_out, __last_code, iteration, __repeat_count).\n"
    "The text should give the solver CONCRETE new guidance for these failures —\n"
    "e.g. suggest a specific construct, decomposition, or worked micro-example.\n"
    "Max 12 lines. Output ONLY the lisp form, nothing else.")))

;; keep only the first balanced form (model may add chatter)
(define (extract-form s)
  (let ((start (str-index-of s "(rlm-register-action")))
    (if (< start 0) ""
      (let ((end (str-index-of (str-substring s start (str-length s)) "\n\n")))
        (if (< end 0) (str-substring s start (str-length s))
          (str-substring s start (+ start end)))))))

(define FORM (extract-form PROPOSAL))

(define (probe)
  ;; compile + call the candidate in a scratch scope; "" = healthy
  (try
    (begin
      (eval (read FORM))
      (let ((e (assoc "probe-ok" CUSTOM-ACTIONS)))  ;; no — probe calls ALL registered
        (if (> (len CUSTOM-ACTIONS) 0)
          (let ((any (car CUSTOM-ACTIONS))) ((car (cdr any)) 2))
          "noreg")))
    (catch e (str-concat "FAILED:" (to-string e)))))

(define PROBE-OUT (probe))
(define OK (and (> (str-length FORM) 30)
                (not (str-contains PROBE-OUT "FAILED"))))

(if OK
  (begin
    (write-file "candidates/action-proposal.lisp"
      (str-concat ";; dream-proposed " (to-string (now)) "\n" FORM "\n"))
    (println "DREAMACTION: banked candidate"))
  (println (str-concat "DREAMACTION: rejected — probe=" (str-substring PROBE-OUT 0 120) " form=" (str-substring FORM 0 80)))
  )
