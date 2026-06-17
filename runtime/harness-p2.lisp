;;; harness-p2.lisp — Minimal Agent Scheduler for WASI Preview 2
;;;
;;; Stateless design: All state stored in OutLayer storage
;;; No mutable globals - functions are pure transforms
;;;
;;; State keys in storage:
;;;   "harness:intentions" - JSON array of intentions
;;;   "harness:budget"     - Budget state
;;;   "harness:inbox"       - Inbox state

;; === Helpers ===

(define (get-default m key default)
  (let ((v (dict/get m key)))
    (if (nil? v) default v)))

;; === Time ===
;; Worker provides NEAR_BLOCK_TIMESTAMP env var

(define (now-ms)
  (let ((ts (env/get "NEAR_BLOCK_TIMESTAMP")))
    (if (or (nil? ts) (= ts "")) 0
      (string->number ts))))

;; === State Management ===
;; All state lives in OutLayer storage

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
;; Scores are integers (0-100), scaled by 100 from real values

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
    ;; Combined score: 70% urgency + 30% efficiency
    ;; Both u and e are 0-100, so score is 0-10000
    ;; We compare scores directly (higher is better)
    (dict/set intent "score" (+ (* 70 u) (* 30 e)))))

;; Find best intention (no full sort needed)
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

;; === Execution ===

(define (execute-action intent)
  (let ((action (get-default intent "action" nil)))
    (if action
      (begin
        (println (str-concat "Executing: " (get-default intent "id" "?")))
        (action))
      (println (str-concat "No action for: " (get-default intent "id" "?"))))))

;; === Intention Lifecycle ===

(define (apply-updates intent updates)
  (cond
    ((nil? updates) intent)
    (else (apply-updates
         (dict/set intent (car (car updates)) (car (cdr (car updates))))
         (cdr updates)))))

(define (update-intention intentions intent-id updates)
  (map (lambda (i)
         (if (equal? (dict/get i "id") intent-id)
           (apply-updates i updates)
           i))
       intentions))

(define (handle-result intent result intentions now)
  (let ((itype (get-default intent "type" "one-shot"))
        (intent-id (get-default intent "id" nil)))
    (cond
      ((equal? itype "perpetual")
       (update-intention intentions intent-id (list (list "last-acted" now))))
      ((equal? itype "completable")
       (update-intention intentions intent-id (list (list "last-acted" now))))
      ((equal? itype "one-shot")
       (filter (lambda (i) (not (equal? (dict/get i "id") intent-id))) intentions))
      ((equal? itype "recurring")
       (update-intention intentions intent-id (list (list "last-run" now))))
      (else intentions))))

;; === Main Entry Point ===
;; Returns the intention that was executed, or nil if none

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
;; Call this to add a new intention

(define (register-intention intent)
  (let ((intentions (load-intentions)))
    (begin
      (save-intentions (append intentions (list intent)))
      intent)))

;; === Boot ===
;; Call once at startup to initialize state

(define (boot)
  (begin
    (println "=== Agent Booting (P2) ===")
    (let ((intentions (load-intentions)))
      (println (str-concat "Intentions loaded: " (to-string (len intentions)))))
    (println "=== Boot Complete ===")
    (quote booted)))