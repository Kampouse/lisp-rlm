export interface Example {
  name: string;
  icon: string;
  source: string;
  target: 'p1' | 'p2' | 'pure';
}

export const examples: Example[] = [
  {
    name: 'Fibonacci',
    icon: '🌀',
    target: 'pure',
    source: `(define (fib n)
  (if (<= n 1)
    n
    (+ (fib (- n 1)) (fib (- n 2)))))

(define (main)
  (fib 10))`,
  },
  {
    name: 'Factorial',
    icon: '❗',
    target: 'pure',
    source: `(define (fact n)
  (if (<= n 1)
    1
    (* n (fact (- n 1)))))

(define (main)
  (fact 12))`,
  },
  {
    name: 'Counter',
    icon: '🔢',
    target: 'p1',
    source: `(memory 1)
(define (get_counter) (near/load "c"))
(define (set_counter val) (near/store "c" val))
(define (new) (set_counter 0))
(define (increment) (set_counter (+ (get_counter) 1)))
(define (get) (near/return (get_counter)))
(export "new" new false)
(export "increment" increment false)
(export "get" get true)`,
  },
  {
    name: 'HTTP Fetch',
    icon: '🌐',
    target: 'p2',
    source: `(define (get-weather)
  (let ((url "https://api.open-meteo.com/v1/forecast?latitude=45.50&longitude=-73.57&current_weather=true"))
    (http-get url)))

(define (main)
  (get-weather))`,
  },
  {
    name: 'P2 Storage',
    icon: '💾',
    target: 'p2',
    source: `;; OutLayer P2 storage demo
;; Uses localStorage in browser, real OutLayer storage on NEAR
(define (main)
  (begin
    (storage-set "count" "42")
    (storage-get "count")))`,
  },
  {
    name: 'Tests',
    icon: '✓',
    target: 'pure',
    source: `;; Test system demo
;; Tests use assert-equal, assert-true, assert-false

(define (add a b) (+ a b))

(test "addition works"
  (assert-equal (add 2 3) 5))

(test "handles zero"
  (assert-equal (add 0 0) 0)
  (assert-equal (add 5 0) 5))

(test "negative numbers"
  (assert-equal (add -1 1) 0)
  (assert-equal (add -5 -3) -8))
`,
  },
];