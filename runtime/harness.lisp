;;; harness.lisp — Layer 2: The Agent's Operating System
;;;
;;; Loaded by the Rust kernel on boot.
;;; Provides the intent loop: score -> rank -> execute -> checkpoint.
;;; Intention-specific code lives in patches/ and is loaded after this.

;; === Helpers ===

(define (get-default m key default)
  (let ((v (dict/get m key)))
    (if (nil? v) default v)))

;; === State ===

(define *intentions* (list))
(define *budget* (dict "daily-limit" 1000 "used" 0))
(define *inbox* (list))

;; === Intention Types ===
;; perpetual  — never done, runs forever
;; completable — has a finish line
;; one-shot   — do once, archive
;; recurring  — runs on schedule

;; === Register ===

(define (register-intention intent)
  (set! *intentions* (append *intentions* (list intent)))
  intent)

;; === Priority Scoring ===

(define (urgency intent)
  (let ((deadline (get-default intent "deadline" nil))
        (last (get-default intent "last-acted" nil))
        (t0 (now)))
    (cond
      ;; Overdue: deadline is a future timestamp, we're past it
      ((and deadline (> t0 deadline)) 1.0)
      ;; Due soon: within 1 hour of deadline
      ((and deadline (< (- deadline t0) 3600000)) 0.9)
      ;; Stale: not acted on in over 1 hour
      ((and last (> (elapsed last) 3600000)) 0.7)
      (t 0.3))))

(define (cost-efficiency intent)
  (let ((cost (get-default intent "cost" 1)))
    (cond
      ((= cost 0) 1.0)
      ((< cost 10) 0.9)
      ((< cost 100) 0.6)
      (t 0.3))))

(define (score-intention intent)
  (let ((u (urgency intent))
        (e (cost-efficiency intent))
        (score (+ (* 0.7 u) (* 0.3 e))))
    (dict/set intent "score" score)))

(define (rank-intentions intentions)
  (map score-intention intentions))

;; === Budget ===

(define (budget-remaining?)
  (< (get-default *budget* "used" 0) (get-default *budget* "daily-limit" 1000)))

(define (budget-spend amount)
  (set! *budget* (dict/set *budget* "used" (+ (get-default *budget* "used" 0) amount))))

;; === Execution ===

(define (execute-action intent)
  (let ((action (get-default intent "action" nil)))
    (if action
      (begin
        (budget-spend (get-default intent "cost" 1))
        (action))
      (println (str-concat "no action for: " (get-default intent "id" "?"))))))

;; === Intention Lifecycle ===

(define (handle-result intent result)
  (let ((itype (get-default intent "type" "one-shot")))
    (cond
      ((equal? itype "perpetual")
       (dict/set intent "last-acted" (now)))
      ((equal? itype "completable")
       (dict/set intent "last-acted" (now)))
      ((equal? itype "one-shot")
       (set! *intentions*
             (filter (lambda (i) (not (equal? (get-default i "id" nil) (get-default intent "id" nil))))
                     *intentions*)))
      ((equal? itype "recurring")
       (dict/set intent "last-run" (now))))))

;; === Scheduler ===

(define (scheduler-run)
  (let ((ranked (rank-intentions *intentions*)))
    (for-each
      (lambda (intent)
        (if (budget-remaining?)
          (let ((result (execute-action intent)))
            (handle-result intent result))))
      ranked)))

;; === Persistence ===

(define (checkpoint)
  (begin
    (save-state "runtime/state/intentions.json" *intentions*)
    (save-state "runtime/state/budget.json" *budget*)
    (save-state "runtime/state/inbox.json" *inbox*)))

(define (restore-state)
  (begin
    (if (file-exists? "runtime/state/intentions.json")
      (set! *intentions* (load-state "runtime/state/intentions.json")))
    (if (file-exists? "runtime/state/budget.json")
      (set! *budget* (load-state "runtime/state/budget.json")))
    (if (file-exists? "runtime/state/inbox.json")
      (set! *inbox* (load-state "runtime/state/inbox.json")))))

;; === Load patches ===

(define (load-patches)
  (if (file-exists? "runtime/patches")
    (let ((files (sort (file/list "runtime/patches"))))
      (for-each
        (lambda (f)
          (if (str-ends-with f ".lisp")
            (load-file (str-concat "runtime/patches/" f))))
        files))
    (println "no patches directory")))

;; === Boot ===

(define (boot)
  (begin
    (println "=== Agent Booting ===")
    (restore-state)
    (load-patches)
    (println (str-concat "Intentions loaded: " (to-string (len *intentions*))))
    (println "=== Boot Complete ===")
    (quote booted)))

;; === Main Loop ===
;; Called by the Rust kernel in a loop. Returns seconds to sleep.

(define (tick)
  (begin
    (scheduler-run)
    (checkpoint)
    60))
