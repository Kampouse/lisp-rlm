;;; harness-p2-minimal.lisp — Minimal test for P2 harness

;; === Boot ===
(define (boot)
  (begin
    (println "=== Agent Booting (P2) ===")
    "booted"))

;; === Tick ===
(define (tick)
  (begin
    (println "Tick")
    60))

;; === Register ===
(define (register-intention intent)
  (begin
    (storage-set "harness:intentions" (list intent))
    intent))

;; === Run entry point ===
(define (run input)
  (boot))