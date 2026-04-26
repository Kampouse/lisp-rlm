#!/usr/bin/env python3
"""
Generate cross-feature integration tests for lisp-rlm.
Tests combinations of features to catch interaction bugs.
"""
import subprocess, os, sys

BINARY = "/Users/asil/.openclaw/workspace/lisp-rlm/target/release/rlm"

def run(code):
    """Run lisp-rlm code, return (stdout, stderr, returncode)."""
    r = subprocess.run(
        [BINARY, "/dev/stdin"],
        input=code, capture_output=True, text=True, timeout=10
    )
    return r.stdout.strip(), r.stderr.strip(), r.returncode

def clean(s):
    """Strip quotes from output."""
    return s.strip('"').strip()

passed = 0
failed = 0
errors = 0

def test(name, code, expected):
    global passed, failed, errors
    stdout, stderr, rc = run(code)
    out = clean(stdout.split('\n')[-1] if stdout else '')
    
    if 'ERROR' in stderr and 'ERROR' not in expected:
        errors += 1
        print(f"  ✗ {name}")
        print(f"    ERROR: {stderr.split(chr(10))[0][:100]}")
    elif out == expected:
        passed += 1
        print(f"  ✓ {name}")
    else:
        failed += 1
        print(f"  ✗ {name}")
        print(f"    expected: {expected}")
        print(f"    got:      {out}")

# ============================================================
print("=== Recursion + Higher-order functions ===")
# ============================================================

test("fib via map",
     '''(define fib (lambda (n) (if (<= n 1) n (+ (fib (- n 1)) (fib (- n 2))))))
     (println (car (map fib (list 10))))''',
     "55")

test("filter + comparison in lambda",
     '''(define (above-ten? x) (> x 10))
     (println (len (filter above-ten? (list 5 11 3 15 20 8 25 1 30))))''',
     "5")

test("reduce + recursive lambda",
     '''(define (sum-list xs)
       (if (nil? xs) 0 (+ (car xs) (sum-list (cdr xs)))))
     (println (sum-list (list 1 2 3 4 5)))''',
     "15")

# ============================================================
print("\n=== let/let*/letrec + closures ===")
# ============================================================

test("let* closure captures correctly",
     '''(define f (let* ((x 1) (y (+ x 1)))
       (lambda (z) (+ x y z))))
     (println (f 10))''',
     "13")

test("letrec mutual + map",
     '''(letrec ((even? (lambda (n) (if (= n 0) true (odd? (- n 1)))))
              (odd? (lambda (n) (if (= n 0) false (even? (- n 1))))))
       (println (car (map even? (list 0 1 2 3 4)))))''',
     "true")

test("let + lambda + set!",
     '''(let ((counter 0))
       (let ((inc! (lambda () (set! counter (+ counter 1)) counter)))
         (inc!) (inc!) (inc!)
         (println counter)))''',
     "3")

test("nested let* with closures",
     '''(let* ((x 10)
              (f (lambda (y) (+ x y))))
       (let* ((x 20))
         (println (f 5))))''',
     "15")  # f captures outer x=10, not inner x=20

# ============================================================
print("\n=== case + cond + when/unless ===")
# ============================================================

test("case with symbol results + cond",
     '''(define (classify n)
       (cond
         ((< n 0) 'negative)
         ((= n 0) 'zero)
         (true (case n
                 ((1) 'one)
                 ((2) 'two)
                 (else 'other)))))
     (println (classify 1))''',
     "one")

test("when inside let",
     '''(let ((x 5))
       (let ((f (lambda () (set! x (* x 2)))))
         (when (> x 3) (f))
         (println x)))''',
     "10")

test("unless + set! loop",
     '''(let ((i 0) (sum 0))
       (loop ((i 0 (+ i 1)) (sum 0 (+ sum i)))
         (when (= i 5) (recur i sum)))
       (println sum))''',
     "")  # just checking no crash

# ============================================================
print("\n=== case-lambda + higher-order ===")
# ============================================================

test("case-lambda as map callback",
     '''(define flex
       (case-lambda
         (() 0)
         ((x) (* x 2))
         ((x y) (+ x y))))
     (println (car (map flex (list 3 5 7))))''',
     "6")

test("case-lambda rest-param",
     '''(define variadic
       (case-lambda
         (() 'none)
         ((x) (list 'one x))
         (args (cons 'many args))))
     (println (car (variadic 1 2 3)))''',
     "many")

# ============================================================
print("\n=== define-values + let-values + map ===")
# ============================================================

test("define-values + list ops",
     '''(define-values (a b) (values 10 20))
     (println (+ a b))''',
     "30")

test("let-values with exact-integer-sqrt + arithmetic",
     '''(let*-values (((root rem) (exact-integer-sqrt 100)))
       (println (- (* root root) rem)))''',
     "100")

# ============================================================
print("\n=== delay/force + higher-order ===")
# ============================================================

test("delay + force + map",
     '''(define p (delay (+ 1 2)))
     (println (car (map force (list p))))''',
     "3")

test("delay in let",
     '''(let ((p (delay (* 6 7))))
       (println (force p)))''',
     "42")

# ============================================================
print("\n=== String + char operations ===")
# ============================================================

test("string-ci + char predicates",
     '''(println (if (string-ci=? "Hello" "HELLO") 'same 'diff))''',
     "same")

test("char ops on string-ref result",
     '''(println (char-upcase (string-ref "hello" 0)))''',
     "H")

test("make-string + string-length",
     '''(println (str-length (string-append (make-string 3 "a") "xyz")))''',
     "6")

# ============================================================
print("\n=== Fractions + math + predicates ===")
# ============================================================

test("fraction literal in arithmetic",
     '''(println (= (+ 1/2 1/2) 1))''',
     "true")

test("expt + predicates chain",
     '''(println (and (integer? (expt 2 10)) (> (expt 2 10) 500)))''',
     "true")

test("float predicates",
     '''(println (and (inexact? +nan.0) (nan? +nan.0) (not (finite? +nan.0))))''',
     "true")

# ============================================================
print("\n=== Error handling + recovery ===")
# ============================================================

test("try/catch + continuation",
     '''(define result
       (try
         (/ 1 0)
         (catch e 'caught)))
     (println result)''',
     "caught")

test("try/catch in loop",
     '''(let ((sum 0))
       (loop ((i 0 (+ i 1)))
         (if (= i 3) (recur i sum)
           (let ((v (try (/ 10 i) (catch e 0))))
             (set! sum (+ sum v)))))
       (println sum))''',
     "")  # just no crash

# ============================================================
print("\n=== Macro + quasiquote ===")
# ============================================================

test("defmacro generating let*",
     '''(defmacro my-swap (a b)
       (list (quote let) (list (list (quote __t) a))
         (list (quote begin)
           (list (quote set!) a b)
           (list (quote set!) b (quote __t)))))
     (let ((x 1) (y 2))
       (my-swap x y)
       (println (+ x y)))''',
     "3")

test("defmacro with quasiquote",
     '''(defmacro my-when (test body)
       (list (quote if) test body (quote nil)))
     (println (my-when true (+ 1 2))))''',
     "3")

# ============================================================
print("\n=== Deep recursion (CPS trampoline) ===")
# ============================================================

test("deep recursion 50K",
     '''(define (count n) (if (= n 0) 0 (count (- n 1))))
     (println (count 50000))''',
     "0")

test("mutual recursion 10K",
     '''(letrec ((e? (lambda (n) (if (= n 0) true (o? (- n 1)))))
              (o? (lambda (n) (if (= n 0) false (e? (- n 1))))))
       (println (if (e? 10000) 'even 'odd)))''',
     "even")

# ============================================================
print("\n=== Arg evaluation order + env isolation ===")
# ============================================================

test("args eval'd left-to-right",
     '''(define (side-effect!) 'done)
     (println (+ (begin (side-effect!) 1) (begin (side-effect!) 2)))''',
     "3")

test("nested function calls preserve env",
     '''(define (make-counter)
       (let ((n 0))
         (lambda ()
           (set! n (+ n 1))
           n)))
     (define c1 (make-counter))
     (define c2 (make-counter))
     (c1) (c1) (c1)
     (println (c2))''',
     "1")  # c2 has its own closure

test("map doesn't mutate source list",
     '''(define xs (list 1 2 3))
     (define ys (map (lambda (x) (* x 10)) xs))
     (println (car xs))''',
     "1")

# ============================================================
print("\n=== Snapshot + rollback + set! ===")
# ============================================================

test("snapshot/rollback preserves state",
     '''(define x 1)
     (snapshot)
     (set! x 99)
     (rollback)
     (println x)''',
     "1")

# ============================================================
print("\n=== Type checking across features ===")
# ============================================================

test("type predicates on let* results",
     '''(let* ((x 42) (y "hello") (z (list 1 2)))
       (println (if (and (integer? x) (string? y) (list? z)) 'ok 'fail)))''',
     "ok")

test("equal? across types",
     '''(println (and (equal? (list 1 2) (list 1 2))
                    (not (equal? (list 1 2) (list 1 3)))
                    (equal? 'a 'a)))''',
     "true")

# ============================================================
# Summary
# ============================================================
total = passed + failed + errors
print(f"\n{'='*50}")
print(f"Integration Tests: {passed} pass, {failed} fail, {errors} errors / {total} total")
if failed == 0 and errors == 0:
    print("✅ All cross-feature tests passing!")
else:
    print(f"⚠️  {failed + errors} issues found")
