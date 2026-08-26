;;; harness-p2.lisp — Minimal Agent Scheduler for WASI Preview 2
;;;
;;; Stateless design: All state stored in OutLayer storage as JSON strings
;;; No mutable globals - functions are pure transforms

;; === Helpers ===

(define (get-default m key default)
  (let ((v (dict/get m key)))
    (if (nil? v) default v)))

;; === Time ===

(define (now-ms)
  0)

;; === JSON round-trip helpers ===

(define (parse-intention json-str)
  (let ((id (json-get "id" json-str))
        (type (json-get "type" json-str))
        (action-type (json-get "action-type" json-str))
        (params-raw (json-get "params" json-str))
        (interval-ms-str (json-get "interval-ms" json-str))
        (priority-str (json-get "priority" json-str))
        (cost-str (json-get "cost" json-str))
        (deadline-str (json-get "deadline" json-str))
        (last-acted-str (json-get "last-acted" json-str)))
    (let ((interval-ms (if (nil? interval-ms-str) nil (string->number interval-ms-str)))
          (priority (if (nil? priority-str) 50 (string->number priority-str)))
          (cost (if (nil? cost-str) 1 (string->number cost-str)))
          (deadline (if (nil? deadline-str) nil (string->number deadline-str)))
          (last-acted (if (nil? last-acted-str) nil (string->number last-acted-str)))
          (query (if (nil? params-raw) "" (json-get "query" params-raw)))
          (account (if (nil? params-raw) "" (json-get "account" params-raw)))
          (method (if (nil? params-raw) "" (json-get "method" params-raw)))
          (args (if (nil? params-raw) "" (json-get "args" params-raw)))
          (prompt (if (nil? params-raw) "" (json-get "prompt" params-raw)))
          (chat-id (if (nil? params-raw) "" (json-get "chat-id" params-raw)))
          (text (if (nil? params-raw) "" (json-get "text" params-raw))))
      (dict "id" id
            "type" type
            "action-type" action-type
            "params" (dict "query" query "account" account "method" method
                           "args" args "prompt" prompt "chat-id" chat-id "text" text)
            "interval-ms" interval-ms
            "priority" priority
            "cost" cost
            "deadline" deadline
            "last-acted" last-acted))))

(define (load-intent-list json-str idx)
  (let ((elem (json-array-get json-str idx)))
    (if (nil? elem) (list)
      (cons (parse-intention elem)
            (load-intent-list json-str (+ idx 1))))))

(define (intent-to-json intent)
  (let ((params (get-default intent "params" (dict)))
        (interval-ms (dict/get intent "interval-ms"))
        (priority (dict/get intent "priority"))
        (cost (dict/get intent "cost"))
        (deadline (dict/get intent "deadline"))
        (last-acted (dict/get intent "last-acted")))
    (str-concat
      "{\"id\":\"" (get-default intent "id" "") "\","
      "\"type\":\"" (get-default intent "type" "one-shot") "\","
      "\"action-type\":\"" (get-default intent "action-type" "") "\","
      "\"params\":{"
        "\"query\":\"" (get-default params "query" "") "\","
        "\"account\":\"" (get-default params "account" "") "\","
        "\"method\":\"" (get-default params "method" "") "\","
        "\"args\":\"" (get-default params "args" "") "\","
        "\"prompt\":\"" (get-default params "prompt" "") "\","
        "\"chat-id\":\"" (get-default params "chat-id" "") "\","
        "\"text\":\"" (get-default params "text" "") "\""
      "},"
      "\"interval-ms\":" (to-string (if (nil? interval-ms) 0 interval-ms)) ","
      "\"priority\":" (to-string (if (nil? priority) 50 priority)) ","
      "\"cost\":" (to-string (if (nil? cost) 1 cost)) ","
      "\"deadline\":" (to-string (if (nil? deadline) 0 deadline)) ","
      "\"last-acted\":" (to-string (if (nil? last-acted) 0 last-acted))
      "}")))

(define (intentions-to-json intentions acc sep)
  (if (nil? intentions)
    (str-concat acc "]")
    (intentions-to-json (cdr intentions)
      (str-concat acc sep (intent-to-json (car intentions)))
      ",")))

;; === State Management ===

(define (load-intentions)
  (let ((data (storage-get "harness:intentions")))
    (if (nil? data) (list)
      (load-intent-list data 0))))

(define (save-intentions intentions)
  (storage-set "harness:intentions" (intentions-to-json intentions "[" "")))

(define (load-budget)
  (let ((data (storage-get "harness:budget")))
    (if (nil? data)
      (dict "daily-limit" 1000 "used" 0)
      (let ((limit-str (json-get "daily-limit" data))
            (used-str (json-get "used" data)))
        (dict "daily-limit"
              (if (nil? limit-str) 1000 (string->number limit-str))
              "used"
              (if (nil? used-str) 0 (string->number used-str)))))))

(define (save-budget budget)
  (let ((limit (dict/get budget "daily-limit"))
        (used (dict/get budget "used")))
    (storage-set "harness:budget"
      (str-concat "{\"daily-limit\":"
        (to-string (if (nil? limit) 1000 limit))
        ",\"used\":"
        (to-string (if (nil? used) 0 used))
        "}"))))

;; === Priority Scoring ===

(define (urgency intent now)
  (let ((deadline (dict/get intent "deadline"))
        (last (dict/get intent "last-acted"))
        (interval (dict/get intent "interval-ms")))
    (let ((effective-interval (if (nil? interval) 3600000 interval)))
      (cond
        ((and deadline (> now deadline)) 100)
        ((and deadline (< (- deadline now) 3600000)) 90)
        ((and last (> (- now last) effective-interval)) 80)
        ((and last (> (- now last) 600000)) 50)
        ((and last (> (- now last) 60000)) 30)
        (else 10)))))

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

(define (execute-action intent)
  (let ((action-type (get-default intent "action-type" nil))
        (params (get-default intent "params" (dict))))
    (if action-type
      (cond
        ((= action-type "near-view")
         (outlayer/view (get-default params "account" "dontcare")
                        (get-default params "method" "dontcare")
                        (get-default params "args" "")))
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
       (update-intention intentions intent-id (list (list "last-acted" now))))
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

;; === Boot (first call only) ===

(define (boot-once)
  (let ((was-booted (storage-get "harness:booted")))
    (if (nil? was-booted)
      (begin
        (storage-set "harness:booted" "true")
        (register-intention (dict
          "id" "near-price-check"
          "type" "recurring"
          "action-type" "web-search"
          "params" (dict "query" "NEAR Protocol price USD today")
          "interval-ms" 600000
          "priority" 50
          "cost" 2))
        true)
      false)))

;; === Run entry point ===

(define (run input)
  (println "=== RUN START ===")
  (boot-once)
  (println "=== AFTER BOOT ===")
  (let ((intentions (load-intentions)))
    (if (nil? intentions)
      (begin (println "No intentions") nil)
      (let ((best (find-best intentions 0)))
        (execute-action best)))))
