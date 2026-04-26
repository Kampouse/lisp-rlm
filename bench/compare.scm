;; Comparative benchmark: Guile 3.0 baseline
;; Run: guile bench/compare.scm

(use-modules (ice-9 format))

(define (time-it label thunk iterations)
  (thunk)
  (let* ((start (get-internal-real-time)))
    (let loop ((i 0))
      (if (< i iterations)
          (begin (thunk) (loop (+ i 1)))))
    (let* ((elapsed (- (get-internal-real-time) start))
           (secs (* 1.0 (/ elapsed internal-time-units-per-second)))
           (per-sec (inexact->exact (round (/ iterations secs)))))
      (format #t "~a: ~a calls in ~,1fms (~a calls/sec)~%" label iterations (* secs 1000.0) per-sec)
      per-sec)))

(define (fib n)
  (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))

(define (get-default m key default)
  (let ((found (assoc-ref m key)))
    (if found found default)))

(define (score-intention item)
  (let* ((urgency (get-default item "urgency" 0.5))
         (cost (get-default item "cost" 1.0))
         (score (* urgency cost)))
    (acons "score" score item)))

(define (map-test lst)
  (map (lambda (x) (* x x)) lst))

(define (filter-test lst)
  (filter (lambda (x) (> x 5)) lst))

(define (sort-test lst)
  (sort lst (lambda (a b) (< a b))))

(define (loop-sum n)
  (let lp ((i 0) (sum 0))
    (if (>= i n) sum (lp (+ i 1) (+ sum i)))))

(define (dict-chain n)
  (let lp ((i 0) (m '()))
    (if (>= i n) m (lp (+ i 1) (acons (number->string i) i m)))))

(format #t "~%=== Guile 3.0 Baseline Benchmarks ===~%~%")

(time-it "fib(30)" (lambda () (fib 30)) 5)

(let ((m (list (cons "a" 1) (cons "b" 2) (cons "c" 3))))
  (time-it "get-default" (lambda () (get-default m "b" 0)) 10000000))

(let ((item (list (cons "urgency" 0.8) (cons "cost" 0.5))))
  (time-it "score-intention" (lambda () (score-intention item)) 1000000))

(let ((lst (iota 100)))
  (time-it "map(100-elem)" (lambda () (map-test lst)) 100000))

(let ((lst (iota 100)))
  (time-it "filter(100-elem)" (lambda () (filter-test lst)) 100000))

(let ((lst (reverse (iota 100))))
  (time-it "sort(100-elem)" (lambda () (sort-test lst)) 10000))

(time-it "loop-sum(1000)" (lambda () (loop-sum 1000)) 100000)

(time-it "dict-chain(100)" (lambda () (dict-chain 100)) 100000)

(format #t "~%=== Done ===~%")
