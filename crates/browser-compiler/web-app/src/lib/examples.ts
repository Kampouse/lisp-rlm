export interface Example {
  name: string;
  icon: string;
  source: string;
  target: 'p1' | 'p2';
}

export const examples: Example[] = [
  {
    name: 'Fibonacci',
    icon: '🌀',
    target: 'p1',
    source: `;; Fibonacci — NEAR Smart Contract
;; Computes the nth Fibonacci number

(define (fib n)
  (if (<= n 1) n
    (+ (fib (- n 1))
       (fib (- n 2)))))

(define (run) (fib 10))`,
  },
  {
    name: 'Counter',
    icon: '🔢',
    target: 'p1',
    source: `;; Simple Counter — NEAR Smart Contract
;; Stores and increments a counter

(define (count-to n)
  (if (<= n 0) 0
    (+ 1 (count-to (- n 1)))))

(define (run) (count-to 5))`,
  },
  {
    name: 'HTTP Fetch',
    icon: '🌐',
    target: 'p2',
    source: `;; HTTP Fetch Service (WASI/OutLayer P2)
;; Fetches Bitcoin price from CoinGecko API

(define (fetch-price)
  (http-get "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd"))

(define (run) (fetch-price))`,
  },
];
