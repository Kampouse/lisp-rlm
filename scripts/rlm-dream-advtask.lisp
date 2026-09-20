;; ADVERSARIAL DREAM TASK — generate homework one notch TOO HARD.
;; Unlike rlm-dream-task.lisp (which practices existing weaknesses with
;; SMALL tasks), this targets the frontier: take a skill the solver
;; can ALMOST do and generate a bigger version that cannot reasonably
;; be solved flat. Forcing stack:
;;   - task prompt embeds [adv] marker + explicit sub-rlm requirement
;;   - __must_decompile flag + FORCE GATE: flat code bounces
;;   - tight budget (max_iterations 5) — flailing fails fast
;;   - REF probe must itself contain a sub-rlm delegation (decomposition
;;     proof: the reference SOLVER decomposes)
;;   - SUB stub answers the delegated sub-task; marker token
;;     distinguishes parent ctx from sub ctx (no stub recursion)
;; Installed as ta_*.lisp via rlm-taskgate.py (separate cap, 24h TTL).
(load-file "rlm_runtime.lisp")
(load-file "scripts/rlm-tasks/policy.lisp")

(define (read-file-or f d)
  (try (read-file f) (catch e d)))

(define (llm-ask p)
  (let ((r (try (llm p) (catch e (str-concat "LLMFAIL:" (to-string e))))))
    (if (str-contains r "LLMFAIL")
      (begin (sleep 8) (try (llm p) (catch e "LLMFAIL:dead")))
      r)))

;; frontier digest: hardest solved / easiest failed, from replay/qlearn
(define FRONTIER (read-file-or "data/rlm/dream/worst-states.txt"
  "argtype| ~~QT~~fold-right|rc2|es2|i2"))

(define PROPOSAL
  (llm-ask (str-concat
    "You are the ADVERSARIAL DREAM layer of a Lisp agent.\n"
    "Its failure frontier (states where it dies):\n\n"
    FRONTIER "\n\n"
    "Pick one frontier skill and write a HARDER task that CANNOT be solved\n"
    "flat. SUBANS is the numeric answer of the delegated sub-task.\n"
    "CRITICAL: before answering, CHECK your arithmetic — REF applied to SUBANS\n"
    "must equal the VERIFY value exactly.\n"
    "in one flat expression — it must be split into two sub-computations.\n"
    "The solver has a small iteration budget, so flailing will fail.\n\n"
    "Answer with EXACTLY these 8 tagged lines, nothing else:\n"
    "STATE: <copy the exact failing state string from above>\n"
    "TASK: ta_<short_name_up_to_20_chars>\n"
    "PROMPT: <60-350 chars, NO double quotes, NO square brackets, asks for a result needing TWO distinct sub-computations combined, store via (rlm-set answer ...), finish via (rlm-set Final true)>\n"
    "SUBTASK: <the delegated sub-problem, 10-80 chars, NO double quotes — what to ask a helper>\n"
    "SUBANS: <just the NUMBER the sub-task yields, digits only>\n"
    "VERIFY: <lisp expression using only the variable a and numbers, true iff a is correct>\n"
    "REF: <one line begin form: (rlm-set e (sub-rlm \"SUBTASK text\")) then combine READING it via (rlm-get e), e.g. (rlm-set answer (+ (rlm-get e) 5)), then (rlm-set Final true)>\n"
    "POISON: <single line: begin form that sets a WRONG answer + (rlm-set Final true), NO sub-rlm>\n\n"
    "Example:\n"
    "STATE: argtype| ~~QT~~fold-right|rc2|es2|i2\n"
    "TASK: ta_foldpair\n"
    "PROMPT: Given (list 4 1 7 2 9), compute the product of the elements greater than 3, then add the count of elements less than 3; store the combined result via (rlm-set answer ...) then (rlm-set Final true).\n"
    "SUBTASK: product of elements greater than 3 in (list 4 1 7 2 9)\n"
    "SUBANS: 252\n"
    "VERIFY: (= a 255)\n"
    "REF: (begin (rlm-set p (sub-rlm \"product of elements greater than 3 in (list 4 1 7 2 9)\")) (rlm-set answer (+ (rlm-get p) 2)) (rlm-set Final true))\n"
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
(define SUBTASK (line-of "SUBTASK" PROPOSAL))
(define SUBANS    (line-of "SUBANS"    PROPOSAL))
(define VERIFY  (line-of "VERIFY"  PROPOSAL))
(define REF     (line-of "REF"     PROPOSAL))
(define POISON  (line-of "POISON"  PROPOSAL))

(define (ok-slug s)
  (and (>= (str-length s) 4) (<= (str-length s) 24)
       (= (str-index-of s "ta_") 0)
       (not (str-contains s "."))
       (not (str-contains s "/"))
       (not (str-contains s "\""))))

;; the marker: injected into the installed PROMPT so the stub can tell
;; parent ctx (contains marker) from sub ctx (does not)
(define MARKER "ADVFRONT")

(define VALID
  (and (ok-slug SLUG)
       (>= (str-length PROMPT) 60)
       (<= (str-length PROMPT) 350)
       (not (str-contains PROMPT "\""))
       (not (str-contains PROMPT "["))
       (not (str-contains PROMPT "\\"))
       (not (str-contains SUBTASK "\\"))
       (> (str-length SUBTASK) 10)
       (<= (str-length SUBTASK) 80)
       (not (str-contains SUBTASK "\""))
       (> (str-length VERIFY) 4)
       (str-contains VERIFY " a ")
       (> (str-length REF) 10)
       (str-contains REF "sub-rlm")
       (str-contains REF "rlm-get")
       (> (str-length POISON) 10)
       (not (str-contains POISON "sub-rlm"))
       (>= (str-length SUBANS) 1)
       (<= (str-length SUBANS) 8)
       (not (= REF POISON))
       (not (str-contains POISON "\""))
       (str-contains FRONTIER STATE)))

;; ---- assemble body: tight budget + must-decompose + verify override ----
(define BODY
  (str-concat
    "(begin\n"
    "  (rlm-set __trace_id \"" SLUG "\")\n"
    "  (rlm-set max_iterations 5)\n"
    "  (rlm-set __must_decompose true)\n"
    "  (define (task-verify a) " VERIFY ")\n"
    "  (run-rlm \"" MARKER " " PROMPT " Solve this by delegating one part via (sub-rlm \\\"" SUBTASK "\\\") and combining.\")\n"
    "  (if (task-verify (rlm-get answer)) nil (rlm-set Final nil))\n"
    "  (write-trace)\n"
    "  (println (str-concat \"RLMDUMP task=" SLUG "\"))\n"
    "  (println (str-concat \"RLMDUMP iterations=\" (to-string (rlm-get iteration))))\n"
    "  (println (str-concat \"RLMDUMP answer=\" (to-string (rlm-get answer))))\n"
    "  (println (str-concat \"RLMDUMP Final=\" (to-string (rlm-get Final))))\n"
    ")"))

;; ---- probes ----
;; stub: parent ctx carries the MARKER → REF; sub ctx does not → SUBSTUB.
;; SUBSTUB solves the delegated sub-task in one flat line (subs are small).
(rlm-set __stub "")
(define SUBSTUB
  (str-concat "(begin (rlm-set answer " SUBANS ") (rlm-set Final true))"))
(define (llm-code ctx)
  (if (str-contains ctx MARKER) (rlm-get __stub_ref) (rlm-get __stub_sub)))

(rlm-set __stub_ref "")
(rlm-set __stub_sub "")

(write-file "/tmp/adv-body-dump.txt" BODY)
(write-file "/tmp/adv-ref.txt" REF)
(write-file "/tmp/adv-subans.txt" SUBANS)
(write-file "/tmp/adv-verify.txt" VERIFY)
(define (probe-run ref-code poison-code)
  (begin
    (rlm-set __stub_ref ref-code)
    (rlm-set __stub_sub SUBSTUB)
    (init-rlm "dream-probe")
    (rlm-set __must_decompose true)
    (eval (read BODY))
    (rlm-get Final)))

(define REF-OUT    (if VALID (probe-run REF POISON) nil))
(define POISON-OUT (if VALID (probe-run POISON POISON) nil))

(define PROBE-OK (and (= REF-OUT true) (not POISON-OUT)))

(define TASKFILE
  (str-concat
    ";; ADVERSARIAL DREAMED TASK — one notch too hard, delegation required\n"
    ";; frontier state: " STATE "\n"
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
    (write-file "candidates/advtask-proposal.lisp" TASKFILE)
    (println "DREAMADV: banked candidate")
    (println (str-concat "DREAMADV: slug=" SLUG " ref=decomposed+completed poison=failed")))
  (begin
    (println (str-concat
      "DREAMADV: rejected — valid=" (to-string VALID)
      " ref=" (to-string REF-OUT) " poison=" (to-string POISON-OUT)
      " slug=" (str-substring SLUG 0 24)))
    (println (str-concat
      "DREAMADV-DBG plen=" (to-string (str-length PROMPT))
      " pquote=" (to-string (str-contains PROMPT "\""))
      " sublen=" (to-string (str-length SUBTASK))
      " subq=" (to-string (str-contains SUBTASK "\""))
      " subans=" SUBANS
      " vlen=" (to-string (str-length VERIFY))
      " vhas_a=" (to-string (str-contains VERIFY " a "))
      " reflen=" (to-string (str-length REF))
      " refsub=" (to-string (str-contains REF "sub-rlm"))
      " pbrack=" (to-string (str-contains PROMPT "["))
      " poislen=" (to-string (str-length POISON))
      " poissub=" (to-string (str-contains POISON "sub-rlm"))
      " poisquote=" (to-string (str-contains POISON "\""))
      
      " subanslen=" (to-string (str-length SUBANS))
      " linked=" (to-string (str-contains FRONTIER STATE))
      " slugok=" (to-string (ok-slug SLUG))
      " sluglen=" (to-string (str-length SLUG))
      " slugidx=" (to-string (str-index-of SLUG "ta_"))
      " vin=" (to-string (str-contains VERIFY SUBANS))
      " refpoison=" (to-string (= REF POISON))))))
