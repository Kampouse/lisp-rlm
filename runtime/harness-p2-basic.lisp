;;; harness-p2-basic.lisp — Basic test for P2 harness

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

;; === Boot ===

(define (boot)
  (begin
    (println "=== Agent Booting (P2) ===")
    (save-intentions (list))
    "booted"))

;; === Tick ===

(define (tick)
  (begin
    (let ((intentions (load-intentions)))
      (if (nil? intentions)
        "no intentions"
        (str-concat "Intentions: " intentions)))))

;; === Run ===

(define (run input)
  (boot))