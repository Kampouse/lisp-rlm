;;; harness.lisp — Layer 2: The Agent's Operating System
;;;
;;; Loaded by the Rust kernel on boot.
;;; Provides the intent loop: score -> rank -> execute -> checkpoint.
;;; Intention-specific code lives in patches/ and is loaded after this.
;;;
;;; Note: Compiled lambdas can't call user-defined functions (get-default etc.)
;;; from inside map/sort/filter. All helpers used in HOF callbacks are inlined.

;; === Helpers ===
;; get-default is kept for top-level use but NOT used inside compiled lambdas

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
;; NOTE: urgency/cost-efficiency use dict/get + nil? directly (not get-default)
;; because they may be called from compiled lambdas inside map

(define (urgency intent)
  (let ((deadline (dict/get intent "deadline"))
        (last (dict/get intent "last-acted"))
        (t0 (now)))
    (cond
      ((and deadline (> t0 deadline)) 1.0)
      ((and deadline (< (- deadline t0) 3600000)) 0.9)
      ((and last (> (elapsed last) 3600000)) 0.7)
      (t 0.3))))

(define (cost-efficiency intent)
  (let ((cost-raw (dict/get intent "cost"))
        (cost (if (nil? cost-raw) 1 cost-raw)))
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

;; Select highest-priority intention (no full sort needed for scheduler)
(define (find-best intentions)
  (if (nil? intentions) nil
    (if (nil? (cdr intentions)) (car intentions)
      (let ((head (car intentions))
            (tail-best (find-best (cdr intentions))))
        (let ((hs (dict/get head "score"))
              (ts (dict/get tail-best "score")))
          (if (> (if (nil? hs) 0 hs) (if (nil? ts) 0 ts))
            head
            tail-best))))))

(define (rank-intentions intentions)
  ;; Score all intentions (sets "score" key on each)
  ;; No full sort — scheduler just picks best each tick
  (map score-intention intentions))

;; === Budget ===
;; NOTE: inline dict/get + nil? checks instead of get-default (compiled lambda scope issue)

(define (budget-remaining?)
  (< (if (nil? (dict/get *budget* "used")) 0 (dict/get *budget* "used"))
     (if (nil? (dict/get *budget* "daily-limit")) 1000 (dict/get *budget* "daily-limit"))))

(define (budget-spend amount)
  (set! *budget* (dict/set *budget* "used" (+ (if (nil? (dict/get *budget* "used")) 0 (dict/get *budget* "used")) amount))))

;; === Execution ===

(define (execute-action intent)
  (let ((action (get-default intent "action" nil)))
    (if action
      (begin
        (budget-spend (get-default intent "cost" 1))
        (try
          (action)
          (catch err
            (println (str-concat "ERROR in action " (get-default intent "id" "?") ": " (to-string err))))))
      (println (str-concat "no action for: " (get-default intent "id" "?"))))))

;; === Intention Lifecycle ===

;; Helper: update an intent in *intentions* by id, applying key-value pairs
;; Uses map to return new list instead of mutating inside lambda
(define (apply-updates intent updates)
  (cond
    ((nil? updates) intent)
    (t (apply-updates
         (dict/set intent (car (car updates)) (car (cdr (car updates))))
         (cdr updates)))))

(define (update-intention intent-id updates)
  (set! *intentions*
    (map (lambda (i)
           (if (equal? (dict/get i "id") intent-id)
             (apply-updates i updates)
             i))
         *intentions*)))

(define (handle-result intent result)
  (let ((itype (get-default intent "type" "one-shot"))
        (intent-id (get-default intent "id" nil)))
    (cond
      ((equal? itype "perpetual")
       (update-intention intent-id (list (list "last-acted" (now)))))
      ((equal? itype "completable")
       (update-intention intent-id (list (list "last-acted" (now)))))
      ((equal? itype "one-shot")
       (set! *intentions*
             (filter (lambda (i) (not (equal? (dict/get i "id") intent-id)))
                     *intentions*)))
      ((equal? itype "recurring")
       (update-intention intent-id (list (list "last-run" (now))))))))

;; === Scheduler ===
;; All helpers defined at top level so bytecode compiler handles them correctly

(define (score-gt a b)
  (> (if (nil? (dict/get a "score")) 0 (dict/get a "score"))
     (if (nil? (dict/get b "score")) 0 (dict/get b "score"))))

(define (pick-best lst current)
  (if (nil? lst) current
    (if (score-gt (car lst) current)
      (pick-best (cdr lst) (car lst))
      (pick-best (cdr lst) current))))

(define (scheduler-run)
  (if (nil? *intentions*) nil
    (begin
      (define best (find-best (rank-intentions *intentions*)))
      (if (budget-remaining?)
        (begin
          (define result (execute-action best))
          (handle-result best result))))))

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
