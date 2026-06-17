;; Simple test for top-level defines
(define *counter* 0)
(define *name* "test")

(define (get-counter)
  *counter*)

(define (increment)
  (set! *counter* (+ *counter* 1))
  *counter*)

(define (run input)
  (increment))