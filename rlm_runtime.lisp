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
    (println "RLM initialized")))

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
      "Generate ONE Lisp expression to execute. You can:\n"
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
        (saved-plen (rlm-get prompt_length)))
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
          sub-result)))))

;; ============================================================
;; 6. ENTRY POINT
;; ============================================================
(define (run-rlm P)
  (begin
    (init-rlm P)
    (rlm-loop)))
