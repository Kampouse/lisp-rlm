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
    source: `(define (handler req)
  (let ((url "https://httpbin.org/get"))
    (let ((response (http-get url)))
      response)))

(define (main)
  (handler "request"))`,
  },
];
