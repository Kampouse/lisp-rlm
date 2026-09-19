;; RLM Runtime - Algorithm 1 from MIT RLM Paper
;; Implemented as Lisp code that runs inside lisp-rlm
;;
;; Usage:
;;   (load-file "rlm_runtime.lisp")
;;   (run-rlm "your task here")
;;
;; Changes from previous version:
;;   - rlm-set/rlm-get now use bare symbols (no quoting needed)
;;   - sub-rlm has proper state isolation (saves/restores parent state)
;;   - Uses load-file pattern

;; ============================================================
;; 1. INITIALIZATION
;; ============================================================
(define (init-rlm P)
  (begin
    (rlm-set prompt P)
    (rlm-set prompt_length (str-length P))
    (rlm-set prompt_preview (str-substring P 0 300))
    (rlm-set Final nil)
    (rlm-set iteration 0)
    (rlm-set max_iterations 15)
    (rlm-set exec_log (list))
    (rlm-set result nil)
    (rlm-set __trace_nodes (list))
    (rlm-set __last_code "")
    (rlm-set __last_out "")
    (rlm-set __repeat_count 0)
    (println "RLM initialized")))

;; ============================================================
;; TRACE EMISSION (Dream-RSI layer: history as replay simulator)
;; One JSON node per iteration; run-rlm writes the world file.
;; ============================================================
(define NL "
")
(define (json-safe s)
  (let ((t (if (string? s) s (to-string s))))
    (str-replace
      (str-replace (str-replace t NL "~~NL~~") "\"" "~~QT~~")
      "\\" "~~BS~~")))
(define (trace-node-json i code ok out)
  (str-concat "{\"i\":" (to-string i)
              ",\"ok\":" (if ok "true" "false")
              ",\"code\":\"" (json-safe code) "\""
              ",\"out\":\"" (json-safe out) "\"}"))
(define (join-nodes lst)
  (if (= (len lst) 0) ""
    (if (= (len lst) 1) (car lst)
      (str-concat (car lst) "," (join-nodes (cdr lst))))))
(define (write-trace)
  (let ((id (json-safe (rlm-get __trace_id)))
        (pol (json-safe (rlm-get __policy)))
        (ts (to-string (now))))
    (begin
      (write-file (str-concat "data/rlm/traces/" (rlm-get __trace_id) ".json")
        (str-concat "{\"task_id\":\"" id "\""
          ",\"ts\":" ts
          ",\"policy\":\"" pol "\""
          ",\"task\":\"" (json-safe (rlm-get prompt)) "\""
          ",\"completed\":" (if (rlm-get Final) "true" "false")
          ",\"iterations\":" (to-string (rlm-get iteration))
          ",\"result\":\"" (json-safe (rlm-get result)) "\""
          ",\"nodes\":[" (join-nodes (rlm-get __trace_nodes)) "]}")))))

;; Anti-repetition escalation: greedy sampling repeats identical code on
;; identical context — so when code repeats, CHANGE the context:
;; full error (untruncated) + the model's own failed code + the failing
;; builtin's doc signature, then demand a strategy change.
(define (error-fn-name out)
  (if (str-contains out "ERROR: ")
    (let ((rest (str-substring out 7 (str-length out))))
      (let ((colon (str-index-of rest ": ")))
        (if (>= colon 0)
          (str-substring rest 0 colon)
          "")))
    ""))

(define (error-fn-doc out)
  (let ((fname (error-fn-name out)))
    (if (not (= fname ""))
      (let ((d (doc fname)))
        (if (str-contains d " — ")
          (str-concat "\nThe function you called is documented as:\n" d "\n")
          ""))
      "")))

;; Concrete hints for the most common misunderstanding: 'sym is a SYMBOL,
;; not a string and not a list. Errors of the "need list/need string" class
;; almost always come from passing 'sym where data was meant.
(define (error-class-hint out)
  (if (str-contains out "need list")
    (str-concat
      "\nTYPE HINT: in lisp-rlm 'sym is a SYMBOL — it is NOT a string and NOT a list.\n"
      "Strings use double quotes: \"precaution\". If a function needs a LIST, build one: (list a b c)\n"
      "or convert: (str->list \"abc\") if available, else iterate the string by index.\n")
    ""))

(define (build-escalation)
  (let ((rc (rlm-get __repeat_count)))
    (if (>= rc 1)
      (let ((out (rlm-get __last_out))
            (fname (error-fn-name (rlm-get __last_out))))
        (str-concat
          "\n\n⚠ YOU REPEATED THE SAME CODE " (to-string rc) "+ TIMES AND IT FAILED.\n"
          "YOUR LAST CODE (this FAILED — do not resubmit it):\n"
          (rlm-get __last_code) "\n\n"
          "FULL ERROR MESSAGE (not truncated):\n" out "\n"
          (error-fn-doc out)
          (error-class-hint out)
          (if (>= rc 2)
            (str-concat
              (if (not (= fname ""))
                (str-concat "\n⛔ " fname " is now BANNED for this task — your code must NOT contain the symbol " fname ".\n")
                "")
              "STRATEGY CHANGE REQUIRED: abandon the failing function entirely; solve it with a different construct (explicit while/dotimes loop with a set! counter, or plain recursion).\n")
            "\nWrite DIFFERENT code than your last attempt.\n")))
      "")))

;; ============================================================
;; 2. CONTEXT BUILDER
;; Only sends metadata + state, never the full prompt
;; ============================================================
(define (rlm-build-context)
  (let ((task (rlm-get prompt))
        (iter (rlm-get iteration))
        (preview (rlm-get prompt_preview))
        (p_len (rlm-get prompt_length))
        (final_val (rlm-get result))
        (log (rlm-get exec_log)))
    (str-concat
      "You are a Recursive Language Model running in a Lisp REPL.\n"
      "Your task: " task "\n\n"
      "Prompt metadata: " (to-string p_len) " chars total, preview:\n"
      (str-substring preview 0 200) "...\n\n"
      "Current iteration: " (to-string iter) "\n"
      "Current result so far: " (to-string final_val) "\n\n"
      "Recent execution log:\n" (to-string log) "\n"
      (build-escalation)
      "\nGenerate ONE Lisp expression to execute. You can:\n"
      "- Use (rlm-set key value) to store results (bare symbol keys, no quoting)\n"
      "- Use (rlm-set Final t) and (rlm-set result <val>) when done\n"
      "- Use (sub-rlm \"sub-task\") to delegate sub-problems\n"
      "- Use (rlm-get prompt) to read the full prompt\n"
      "- Use string functions to slice/inspect the prompt\n"
      "Return ONLY valid Lisp code.")))

;; ============================================================
;; 3. SINGLE STEP
;; Snapshot -> Generate -> Execute -> Log or Rollback
;; ============================================================
(define (rlm-step)
  (begin
    (snapshot)
    (let ((ctx (rlm-build-context)))
      (let ((code (llm-code ctx)))
        (let ((exec-result
                (try
                  (eval (read code))
                  (catch e
                    (begin
                      (rollback)
                      (str-concat "ERROR: " (to-string e)))))))
          (let ((is-error (str-contains (to-string exec-result) "ERROR:")))
            ;; anti-repetition tracking: consecutive identical generations
            (if (equal? code (rlm-get __last_code))
              (rlm-set __repeat_count (+ (rlm-get __repeat_count) 1))
              (rlm-set __repeat_count 0))
            (rlm-set __last_code code)
            (rlm-set __last_out (to-string exec-result))
            (rlm-set __trace_nodes
              (append (rlm-get __trace_nodes)
                (list (trace-node-json (+ (rlm-get iteration) 1) code (not is-error) exec-result))))
            (rlm-set exec_log
              (append (rlm-get exec_log)
                (list (str-concat "iter " (to-string (rlm-get iteration))
                       ": " (str-substring (to-string exec-result) 0 200)))))
            (rlm-set iteration (+ (rlm-get iteration) 1))
            (if is-error
              (println (str-concat "[RLM " (to-string (rlm-get iteration)) "] ERR - retrying"))
              (println (str-concat "[RLM " (to-string (rlm-get iteration)) "] OK")))
            exec-result))))))

;; ============================================================
;; 4. MAIN LOOP
;; Runs until Final is set or max iterations reached
;; ============================================================
(define (rlm-loop)
  (if (rlm-get Final)
    (begin
      (println (str-concat "RLM completed in " (to-string (rlm-get iteration)) " iterations"))
      (rlm-get result))
    (if (> (rlm-get iteration) (rlm-get max_iterations))
      (begin
        (println "RLM: max iterations reached")
        (rlm-get result))
      (begin
        (rlm-step)
        (rlm-loop)))))

;; ============================================================
;; 5. SUB-RLM (recursive sub-problem solving)
;; Isolated state: saves all parent rlm_state keys, runs sub-task,
;; then restores parent state completely.
;; ============================================================
(define (sub-rlm sub-prompt)
  ;; Save parent state
  (let ((saved-prompt (rlm-get prompt))
        (saved-iter (rlm-get iteration))
        (saved-result (rlm-get result))
        (saved-log (rlm-get exec_log))
        (saved-final (rlm-get Final))
        (saved-max (rlm-get max_iterations))
        (saved-preview (rlm-get prompt_preview))
        (saved-plen (rlm-get prompt_length))
        (saved-trace (rlm-get __trace_nodes))
        (saved-tid (rlm-get __trace_id))
        (saved-pol (rlm-get __policy)))
    ;; Run sub-task with clean state
    (begin
      (init-rlm sub-prompt)
      (rlm-set max_iterations 5)
      (let ((sub-result (rlm-loop)))
        ;; Restore parent state
        (begin
          (rlm-set prompt saved-prompt)
          (rlm-set iteration saved-iter)
          (rlm-set result saved-result)
          (rlm-set exec_log saved-log)
          (rlm-set Final saved-final)
          (rlm-set max_iterations saved-max)
          (rlm-set prompt_preview saved-preview)
          (rlm-set prompt_length saved-plen)
          (rlm-set __trace_nodes saved-trace)
          (rlm-set __trace_id saved-tid)
          (rlm-set __policy saved-pol)
          sub-result)))))

;; ============================================================
;; 6. ENTRY POINT
;; ============================================================
(define (run-rlm P)
  (begin
    (init-rlm P)
    (let ((res (rlm-loop)))
      (begin
        (write-trace)
        res))))
