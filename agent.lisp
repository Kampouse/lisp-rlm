;; ============================================================
;; agent.lisp — Self-modifying agent definition
;; Loaded on boot by lisp-rlm runtime
;; ============================================================

;; --- Bootstrap: load learned strategies if they exist ---
(if (file-exists? "strategies.lisp")
  (load-file "strategies.lisp"))

;; --- Load memory from last session ---
(if (file-exists? "memory/state.lisp")
  (load-file "memory/state.lisp"))

;; ============================================================
;; IDENTITY
;; ============================================================
(define agent-name "Gork")
(define agent-version "0.1.0")

;; ============================================================
;; ROUTING — decide how to handle each message
;; ============================================================
(define (route message)
  (define intent (llm (str-concat
    "Classify this message in one word: question, code, research, analysis, system, chat\n"
    "Message: " message)))
  
  (cond
    ((str-contains intent "code")
     (handle-code message))
    ((str-contains intent "research")
     (handle-research message))
    ((str-contains intent "analysis")
     (handle-analysis message))
    ((str-contains intent "system")
     (handle-system message))
    (true
     (handle-chat message))))

;; ============================================================
;; HANDLERS — one per intent type
;; ============================================================

(define (handle-code message)
  ;; Generate code, verify, save, run
  (define code (rlm-write message "/tmp/generated.lisp"))
  (define result (load-file "/tmp/generated.lisp"))
  (learn "code" message result true)
  result)

(define (handle-research message)
  ;; Find sources, chunk, batch analyze, synthesize
  (define query (llm (str-concat "Extract search terms: " message)))
  (define sources (llm (str-concat "What URLs or files should I read for: " query)))
  
  ;; Collect content from sources
  (define content (reduce str-concat "" 
    (map (lambda (url) 
      (try (http-get url) (lambda (e) ""))) 
         (str-split sources "\n"))))
  
  ;; Chunk and batch
  (define chunks (str-chunk content 8))
  (define prompts (map (lambda (c) 
    (str-concat "Extract key findings from this research:\n" c)) chunks))
  (define summaries (llm-batch prompts))
  
  ;; Synthesize
  (define answer (llm (str-concat 
    "Synthesize into a clear answer for: " message "\n\nFindings:\n" 
    (str-join "\n" summaries))))
  (learn "research" message answer true)
  (final answer))

(define (handle-analysis message)
  ;; Multi-step RLM with self-verification
  (define result (rlm (str-concat
    "Analyze step by step. Use show-vars to track your progress.\n" message)))
  (define verified (llm (str-concat
    "Is this analysis correct? Answer YES or NO:\n" (to-string result))))
  
  (if (str-contains verified "NO")
    (begin
      (define fixed (rlm (str-concat
        "Previous analysis was wrong. Fix it:\n" message "\nCritique: " verified)))
      (learn "analysis" message fixed true)
      (final fixed))
    (begin
      (learn "analysis" message result true)
      (final result))))

(define (handle-system message)
  ;; Self-modification commands
  (cond
    ;; Learn a new strategy
    ((str-contains message "learn ")
     (define strategy (llm (str-concat 
       "Write a reusable Lisp function for this pattern:\n" message)))
     (append-file "strategies.lisp" (str-concat strategy "\n"))
     (load-file "strategies.lisp")
     (final (str-concat "Learned and loaded: " (str-substring strategy 0 100))))
    
    ;; Show current state
    ((str-contains message "state")
     (final (str-concat "Vars:\n" (show-vars) "\nTokens: " (to-string (rlm-tokens)))))
    
    ;; Modify own routing
    ((str-contains message "add route")
     (define new-route (rlm-write 
       (str-concat "Write a new handler function and routing rule for this intent:\n" message)
       "/tmp/new_route.lisp"))
     (load-file "/tmp/new_route.lisp")
     (final "Route added and loaded"))
    
    (true
     (final (llm message)))))

(define (handle-chat message)
  (final (llm message)))

;; ============================================================
;; LEARNING — save successful strategies
;; ============================================================
(define (learn task-type task result success)
  (if success
    (begin
      (rlm-set "total_tasks" (+ (rlm-get "total_tasks") 1))
      (rlm-set "successes" (+ (rlm-get "successes") 1))
      (define entry (str-concat 
        ";; Learned " task-type " at " (to-string (now)) "\n"
        ";; Task: " (str-substring task 0 100) "\n"
        ";; Result: " (str-substring (to-string result) 0 100) "\n\n"))
      (append-file "memory/learned.lisp" entry))))

;; ============================================================
;; HEARTBEAT — periodic checks
;; ============================================================
(define (heartbeat)
  ;; Check if there are new messages
  (define inbox (check-inbox))
  (if (> (len inbox) 0)
    (map (lambda (msg)
      (send-response (route msg))) inbox)
    
    ;; No messages — do background work
    (begin
      ;; Consolidate memory if it's been a while
      (if (> (- (now) (rlm-get "last_memory_check")) 3600)
        (begin
          (define memory (llm 
            (str-concat "Consolidate these learned strategies into better ones:\n"
              (read-file "memory/learned.lisp")))
          (write-file "strategies.lisp" memory)
          (load-file "strategies.lisp")
          (rlm-set "last_memory_check" (now))))
      
      ;; Check on ongoing tasks
      (define tasks (rlm-get "active_tasks"))
      (if (> (len tasks) 0)
        (map process-task tasks)
        nil))))

;; ============================================================
;; MAIN LOOP — runs forever
;; ============================================================
(define (run)
  (heartbeat)
  (run))  ;; tail call — trampolined by the runtime

;; ============================================================
;; BOOT — initialize state on first run
;; ============================================================
(if (not (rlm-get "booted"))
  (begin
    (rlm-set "booted" true)
    (rlm-set "total_tasks" 0)
    (rlm-set "successes" 0)
    (rlm-set "last_memory_check" 0)
    (rlm-set "active_tasks" (list))
    (rlm-set "version" agent-version)
    (println (str-concat agent-name " v" agent-version " booted"))))

;; Start
(run)
