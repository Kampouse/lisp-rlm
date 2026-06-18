;;; harness-p2.lisp — Minimal Agent Scheduler for WASI Preview 2
;;;
;;; Stateless design: All state stored in OutLayer storage
;;; No mutable globals - functions are pure transforms
;;;
;;; State keys in storage:
;;;   "harness:intentions" - JSON array of intentions
;;;   "harness:budget"     - Budget state
;;;   "harness:inbox"      - Inbox state
;;;
;;; Actions are identified by string type, not lambdas:
;;;   "near-view" - call near/view
;;;   "http-get"  - HTTP GET request
;;;   "http-post" - HTTP POST request
;;;   "web-search" - web search via ZAI MCP

;; === Helpers ===

(define (get-default m key default)
  (let ((v (dict/get m key)))
    (if (nil? v) default v)))

;; === Time ===

(define (now-ms)
  (let ((ts (env/get "NEAR_BLOCK_TIMESTAMP")))
    (if (or (nil? ts) (= ts "")) 0
      (string->number ts))))

;; === State Management ===

(define (load-intentions)
  (let ((data (storage-get "harness:intentions")))
    (if (nil? data) (list) data)))

(define (save-intentions intentions)
  (storage-set "harness:intentions" intentions))

(define (load-budget)
  (let ((data (storage-get "harness:budget")))
    (if (nil? data) (dict "daily-limit" 1000 "used" 0) data)))

(define (save-budget budget)
  (storage-set "harness:budget" budget))

;; === Priority Scoring ===

(define (urgency intent now)
  (let ((deadline (dict/get intent "deadline"))
        (last (dict/get intent "last-acted")))
    (cond
      ((and deadline (> now deadline)) 100)
      ((and deadline (< (- deadline now) 3600000)) 90)
      ((and last (> (- now last) 3600000)) 70)
      (else 30))))

(define (cost-efficiency intent)
  (let ((cost-raw (dict/get intent "cost"))
        (cost (if (nil? cost-raw) 1 cost-raw)))
    (cond
      ((= cost 0) 100)
      ((< cost 10) 90)
      ((< cost 100) 60)
      (else 30))))

(define (score-intention intent now)
  (let ((u (urgency intent now))
        (e (cost-efficiency intent)))
    (dict/set intent "score" (+ (* 70 u) (* 30 e)))))

(define (find-best intentions now)
  (if (nil? intentions) nil
    (if (nil? (cdr intentions))
      (score-intention (car intentions) now)
      (let ((head (score-intention (car intentions) now))
            (tail-best (find-best (cdr intentions) now)))
        (let ((hs (dict/get head "score"))
              (ts (dict/get tail-best "score")))
          (if (> (if (nil? hs) 0 hs) (if (nil? ts) 0 ts))
            head
            tail-best))))))

;; === Budget ===

(define (budget-remaining? budget)
  (< (if (nil? (dict/get budget "used")) 0 (dict/get budget "used"))
     (if (nil? (dict/get budget "daily-limit")) 1000 (dict/get budget "daily-limit"))))

(define (budget-spend budget amount)
  (dict/set budget "used" (+ (if (nil? (dict/get budget "used")) 0 (dict/get budget "used")) amount)))

;; === Action Dispatch ===
;;; Actions are identified by "action-type" string in intention
;;; Supported types: "near-view", "http-get", "http-post", "web-search", "ai-chat", "send-telegram"

(define (execute-action intent)
  (let ((action-type (get-default intent "action-type" nil))
        (params (get-default intent "params" (dict))))
    (if action-type
      (cond
        ((= action-type "near-view")
         (outlayer/view (get-default params "account" "dontcare")
                        (get-default params "method" "dontcare")
                        (get-default params "args" "")))
        ((= action-type "http-get")
         (http-get (get-default params "url" "https://example.com")))
        ((= action-type "http-post")
         (http-post (get-default params "url" "https://example.com")
                    (get-default params "body" "")))
        ((= action-type "web-search")
         (web-search (get-default params "query" "")))
        ((= action-type "ai-chat")
         (ai-chat (get-default params "prompt" "")))
        ((= action-type "send-telegram")
         (send-telegram (get-default params "chat-id" "5125145880")
                        (get-default params "text" "")))
        (else (str-concat "Unknown action-type: " action-type)))
      "No action-type specified")))

;; === Intention Lifecycle ===

(define (apply-updates intent updates)
  (cond
    ((nil? updates) intent)
    (else (apply-updates
         (dict/set intent (car (car updates)) (car (cdr (car updates))))
         (cdr updates)))))

(define (update-intention intentions intent-id updates)
  (map (lambda (i)
         (if (= (dict/get i "id") intent-id)
           (apply-updates i updates)
           i))
       intentions))

(define (handle-result intent result intentions now)
  (let ((itype (get-default intent "type" "one-shot"))
        (intent-id (get-default intent "id" nil)))
    (cond
      ((= itype "perpetual")
       (update-intention intentions intent-id (list (list "last-acted" now))))
      ((= itype "completable")
       (update-intention intentions intent-id (list (list "last-acted" now))))
      ((= itype "one-shot")
       (filter (lambda (i) (not (= (dict/get i "id") intent-id))) intentions))
      ((= itype "recurring")
       (update-intention intentions intent-id (list (list "last-run" now))))
      (else intentions))))

;; === Main Entry Point ===

(define (tick)
  (let ((now (now-ms))
        (intentions (load-intentions))
        (budget (load-budget)))
    (if (nil? intentions)
      (begin
        (println "No intentions to execute")
        nil)
      (let ((best (find-best intentions now)))
        (if (budget-remaining? budget)
          (let ((result (execute-action best))
                (new-intentions (handle-result best result intentions now))
                (new-budget (budget-spend budget (get-default best "cost" 1))))
            (begin
              (save-intentions new-intentions)
              (save-budget new-budget)
              best))
          (begin
            (println "Budget exhausted")
            nil))))))

;; === Register Intention ===

(define (register-intention intent)
  (let ((intentions (load-intentions)))
    (begin
      (save-intentions (append intentions (list intent)))
      intent)))

;; === Boot (first call only) — uses storage to track ===

(define (boot-once)
  (let ((was-booted (storage-get "harness:booted")))
    (if (nil? was-booted)
      (begin
        (storage-set "harness:booted" "true")
        true)
      false)))

;; === AI Agent Tick ===

(define (ai-tick)
  (let ((inbox (storage-get "harness:inbox")))
    (if (nil? inbox)
      (begin
        (println "AI tick: no inbox, checking intentions")
        (tick))
      (let* ((p1 "You are an autonomous NEAR blockchain agent with tools: web-search, send-telegram, http-get, http-post, near-view. When asked a question, first search for info, then respond concisely.")
             (p2 "Message: ")
             (prompt (str-concat p1 " " p2 inbox)))
        (let ((ai-response (ai-chat prompt)))
          ai-response)))))

;; === Run entry point ===

(define (run input)
  (boot-once)
  (if (nil? input)
    (tick)
    (ai-tick)))