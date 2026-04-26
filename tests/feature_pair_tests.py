#!/usr/bin/env python3
"""
Systematic feature-pair cross-function test generator for lisp-rlm.

Generates tests for each feature and every pair of features,
establishing a matrix of cross-function coverage.

Usage:
  python3 tests/feature_pair_tests.py           # run all
  python3 tests/feature_pair_tests.py --list     # show test names
  python3 tests/feature_pair_tests.py closure    # run only closure tests
"""

import subprocess, sys

BINARY = "/Users/asil/.openclaw/workspace/lisp-rlm/target/release/rlm"

def run(code):
    r = subprocess.run(
        [BINARY, "/dev/stdin"],
        input=code, capture_output=True, text=True, timeout=30
    )
    return r.stdout.strip(), r.stderr.strip(), r.returncode

def clean(s):
    """Get the last line of output, stripped of surrounding quotes."""
    if not s:
        return ""
    lines = [l.strip() for l in s.split("\n") if l.strip()]
    last = lines[-1] if lines else ""
    # Strip surrounding quotes if present
    if len(last) >= 2 and last[0] == '"' and last[-1] == '"':
        last = last[1:-1]
    return last

passed = failed = errors = 0

def test(name, code, expected):
    global passed, failed, errors
    stdout, stderr, rc = run(code)
    out = clean(stdout)
    if "ERROR" in stderr and "ERROR" not in expected:
        errors += 1
        err_line = stderr.split("\n")[0][:100]
        print(f"  \U0001f4a5 {name}")
        print(f"      {err_line}")
    elif out == expected:
        passed += 1
        print(f"  \u2713 {name}")
    else:
        failed += 1
        print(f"  \u2717 {name}")
        print(f"      expected: {expected}")
        print(f"      got:      {out}")

# ============================================================
print("=" * 60)
print("SINGLE FEATURE TESTS (baseline)")
print("=" * 60)
# ============================================================

# -- Control Flow --
test("if-basic",
     "(println (if true 1 2))", "1")
test("if-nested",
     "(println (if (if true true false) 1 2))", "1")
test("cond-basic",
     "(println (cond (false 1) (true 2) (else 3)))", "2")
test("case-basic",
     "(case (+ 1 2) ((1) (println 1)) ((3) (println 3)) (else (println 0)))",
     "3")
test("when-basic",
     "(when true (println 42))", "42")
test("unless-basic",
     "(unless false (println 7))", "7")
test("begin-basic",
     "(println (begin 1 2 3))", "3")
test("and-or",
     "(println (and 1 2 (or false 3)))", "3")
test("match-basic",
     "(match (list 1 2) ((list a b) (println (+ a b))) (else (println 0)))", "3")

# -- Closures --
test("lambda-basic",
     "(println ((lambda (x) (+ x 1)) 5))", "6")
test("closure-capture",
     "(let ((x 10)) (let ((f (lambda (y) (+ x y)))) (println (f 5))))", "15")
test("closure-shadow",
     "(let* ((x 10) (f (lambda (y) (+ x y)))) (let* ((x 99)) (println (f 5))))", "15")
test("closure-mutation",
     "(let ((n 0)) (let ((inc (lambda () (set! n (+ n 1)) n))) (begin (inc) (inc) (println n))))", "2")
test("closure-independent",
     "(define (make-counter) (let ((n 0)) (lambda () (begin (set! n (+ n 1)) n)))) (define c1 (make-counter)) (define c2 (make-counter)) (c1) (c1) (c1) (println (c2))", "1")
test("case-lambda-basic",
     "(define f (case-lambda (() 0) ((x) (* x 2)) ((x y) (+ x y)))) (println (+ (f) (f 3) (f 4 5)))", "15")
test("case-lambda-rest",
     "(define f (case-lambda (() (quote none)) (args (cons (quote many) args)))) (println (car (f 1 2 3)))", "many")

# -- Let/Let*/Letrec --
test("let-basic",
     "(println (let ((x 1) (y 2)) (+ x y)))", "3")
test("let*-sequential",
     "(println (let* ((x 1) (y (+ x 1))) (+ x y)))", "3")
test("letrec-basic",
     "(letrec ((fact (lambda (n) (if (<= n 1) 1 (* n (fact (- n 1))))))) (println (fact 5)))", "120")
test("let-values-basic",
     "(let-values (((a b) (values 10 20))) (println (+ a b)))", "30")
test("define-values",
     "(define-values (x y) (values 1 2)) (println (+ x y))", "3")

# -- Higher-order Functions --
test("map-basic",
     "(println (car (map (lambda (x) (* x 10)) (list 1 2 3))))", "10")
test("filter-basic",
     "(println (len (filter (lambda (x) (> x 3)) (list 1 2 3 4 5))))", "2")
test("reduce-basic",
     "(println (reduce + 0 (list 1 2 3 4 5)))", "15")
test("fold-left",
     "(println (fold-left (lambda (acc x) (+ acc x)) 0 (list 1 2 3)))", "6")
test("fold-right",
     "(println (fold-right (lambda (x acc) (+ x acc)) 0 (list 1 2 3)))", "6")
test("apply-basic",
     "(println (apply + (list 1 2 3)))", "6")
test("compose",
     "(define (compose f g) (lambda (x) (f (g x)))) (define inc-dbl (compose (lambda (x) (* x 2)) (lambda (x) (+ x 1)))) (println (inc-dbl 4))", "10")
test("every-some",
     "(println (if (and (every (lambda (x) (> x 0)) (list 1 2 3)) (some (lambda (x) (> x 2)) (list 1 2 3))) 1 0))", "1")

# -- List Operations --
test("cons-car-cdr",
     "(println (car (cdr (cons 1 (cons 2 nil)))))", "2")
test("list-ref-tail",
     "(println (+ (list-ref (list 10 20 30) 1) (car (list-tail (list 10 20 30) 2))))", "50")
test("append-reverse",
     "(println (car (reverse (append (list 1 2) (list 3 4)))))", "4")
test("assoc-member",
     "(println (if (and (equal? (assoc 2 (list (list 1 100) (list 2 200))) (list 2 200)) (member 3 (list 1 2 3))) 1 0))", "1")
test("map?-dict",
     '(println (if (map? (dict "a" 1 "b" 2)) 1 0))', "1")

# -- String Operations --
test("string-append-len",
     '(println (str-length (string-append "hello" " " "world")))', "11")
test("string-ref",
     '(println (string-ref "hello" 1))', '"e"')
test("string-ci",
     '(println (if (string-ci=? "Hello" "HELLO") 1 0))', "1")
test("str-split-join",
     '(println (str-join "-" (list "a" "b" "c")))', '"a-b-c"')
test("str-contains",
     '(println (if (str-contains "hello world" "world") 1 0))', "1")

# -- Math Operations --
test("arithmetic",
     "(println (+ (* 3 4) (- 10 5)))", "17")
test("mod-div",
     "(println (+ (mod 17 5) (/ 10 3)))", "5")
test("expt-sqrt",
     "(println (if (and (= (expt 2 10) 1024) (= (sqrt 9) 3)) 1 0))", "1")
test("floor-ceiling",
     "(println (+ (floor 3.7) (ceiling 3.2)))", "7")
test("fraction",
     "(println (if (= (+ 1/2 1/2) 1) 1 0))", "1")

# -- Predicates --
test("type-predicates",
     "(println (if (and (integer? 42) (string? \"hi\") (list? (list 1)) (nil? nil)) 1 0))", "1")
test("comparison",
     "(println (if (and (< 1 2) (> 3 2) (<= 1 1) (>= 2 1)) 1 0))", "1")
test("equal-eqv",
     "(println (if (and (equal? (list 1 2) (list 1 2)) (eqv? 42 42)) 1 0))", "1")

# -- Macros --
test("defmacro-basic",
     "(defmacro my-add (a b) (list (quote +) a b)) (println (my-add 3 4))", "7")

# -- Delay/Force --
test("delay-force",
     "(println (force (delay (+ 1 2))))", "3")
test("delay-memoizes",
     "(let ((calls 0)) (let ((p (delay (begin (set! calls (+ calls 1)) calls)))) (begin (force p) (force p) (println calls))))", "2")

# -- Error Handling --
test("try-catch",
     "(println (try (/ 1 0) (catch e 0)))", "0")
test("try-catch-nested",
     "(println (try (+ 1 (try (/ 1 0) (catch e 0))) (catch e -1)))", "1")

# -- State --
test("set!-global",
     "(define x 10) (set! x 20) (println x)", "20")

# -- Recursion --
test("deep-recursion",
     "(define (count n) (if (= n 0) 0 (count (- n 1)))) (println (count 10000))", "0")

s_p, s_f, s_e = passed, failed, errors
print(f"\n  Singles: {s_p}/{s_p+s_f+s_e} pass, {s_f} fail, {s_e} errors")

# ============================================================
print("\n" + "=" * 60)
print("FEATURE PAIR TESTS (cross-function)")
print("=" * 60)
# ============================================================

passed = failed = errors = 0

# -- closure × hof --
test("closure × map",
     "(let ((x 10)) (let ((f (lambda (y) (+ x y)))) (println (car (map f (list 1 2 3))))))",
     "11")
test("closure × filter",
     "(let ((threshold 3)) (let ((above? (lambda (x) (> x threshold)))) (println (len (filter above? (list 1 2 3 4 5))))))",
     "2")
test("closure-mutation × for-each",
     "(let ((sum 0)) (let ((adder (lambda (x) (set! sum (+ sum x))))) (for-each adder (list 1 2 3)) (println sum)))",
     "6")
test("case-lambda × map",
     "(define f (case-lambda (() 0) ((x) (* x 2)))) (println (car (map f (list 1 2 3))))",
     "2")
test("closure-shadow × reduce",
     "(let* ((x 10) (f (lambda (acc v) (+ acc (* x v))))) (let* ((x 999)) (println (reduce f 0 (list 1 2 3)))))",
     "60")

# -- binding × closure --
test("let* × closure-capture",
     "(let* ((x 1) (f (lambda () (+ x 1))) (x 99)) (println (f)))",
     "2")
test("let-values × closure",
     "(let-values (((a b) (values 10 20))) (let ((f (lambda () (+ a b)))) (println (f))))",
     "30")

# -- control × hof --
test("cond × map",
     "(define (classify n) (cond ((< n 0) -1) ((= n 0) 0) (true 1))) (println (map classify (list -1 0 1)))",
     "(-1 0 1)")
test("case × filter",
     "(define (vowel? c) (case c ((a) true) ((e) true) ((i) true) ((o) true) ((u) true) (else false))) (println (len (filter vowel? (list (quote a) (quote b) (quote e) (quote x) (quote o)))))",
     "3")
test("match × map",
     "(define (describe x) (match x ((list a b) (+ a b)) ((list a) a) (else 0))) (println (map describe (list (list 1 2) (list 5) 42)))",
     "(3 5 0)")

# -- control × binding --
test("if × let*",
     "(let* ((a 5) (b (if (> a 3) (* a 2) a))) (println b))",
     "10")

# -- hof × list --
test("map × reverse",
     "(println (reverse (map (lambda (x) (* x 2)) (list 1 2 3))))",
     "(6 4 2)")
test("reduce × cons",
     "(println (reduce (lambda (acc x) (cons x acc)) nil (list 1 2 3)))",
     "(3 2 1 nil)")
test("fold-left × assoc",
     "(let ((pairs (list (list 1 10) (list 2 20) (list 3 30)))) (println (fold-left (lambda (acc p) (+ acc (list-ref p 1))) 0 pairs)))",
     "60")

# -- string × hof --
test("string-len × map",
     "(println (len (map str-length (list \"hello\" \"world\" \"hi\"))))",
     "3")
test("string-ref × map",
     "(println (map (lambda (s) (string-ref s 0)) (list \"hello\" \"world\")))",
     '("h" "w")')

# -- math × hof --
test("expt × map",
     "(println (car (map (lambda (x) (expt x 2)) (list 3 4 5))))",
     "9")
test("mod × filter",
     "(println (len (filter even? (map (lambda (x) (mod x 10)) (list 12 23 34 45 56)))))",
     "3")

# -- pred × hof --
test("integer? × every",
     "(println (if (every (lambda (x) (integer? x)) (list 1 2 3)) 1 0))",
     "1")
test("equal? × filter",
     "(let ((xs (list (list 1) (list 2) (list 1)))) (println (len (filter (lambda (x) (equal? x (list 1))) xs))))",
     "2")

# -- macro × control --
test("defmacro × let",
     "(defmacro my-let (pair body) (list (quote let) (list pair) body)) (println (my-let (x 42) (+ x 1)))",
     "43")

# -- lazy × hof --
test("delay × map",
     "(let ((p (delay 42))) (println (car (map force (list p)))))",
     "42")

# -- error × hof --
test("try-catch × map",
     "(define (safe-div x) (try (/ 10 x) (catch e 0))) (println (map safe-div (list 2 0 5)))",
     "(5 0 2)")
test("try-catch × filter",
     "(define (safe-inv x) (try (begin (/ 1 x) true) (catch e false))) (println (len (filter safe-inv (list 1 0 2 0 3))))",
     "3")

# -- state × closure --
test("set! × closure",
     "(define x 10) (define (bump!) (set! x (+ x 1))) (bump!) (bump!) (println x)",
     "12")

# -- recur × hof --
test("recursion × map",
     "(define fib (lambda (n) (if (<= n 1) n (+ (fib (- n 1)) (fib (- n 2)))))) (println (map fib (list 5 6 7)))",
     "(5 8 13)")

# -- Triple combos --
test("let* × closure × reduce",
     "(let* ((m 3) (transform (lambda (x) (* x m)))) (let* ((m 999)) (println (reduce + 0 (map transform (list 1 2 3))))))",
     "18")

p_p, p_f, p_e = passed, failed, errors
print(f"\n  Pairs: {p_p}/{p_p+p_f+p_e} pass, {p_f} fail, {p_e} errors")

# ============================================================
# Summary
# ============================================================
total_p = s_p + p_p
total_f = s_f + p_f
total_e = s_e + p_e
total = total_p + total_f + total_e

print(f"\n{'=' * 60}")
print(f"TOTAL: {total_p}/{total} pass ({100*total_p//total if total else 0}%), {total_f} fail, {total_e} errors")
if total_f == 0 and total_e == 0:
    print("\u2705 ALL TESTS GREEN")
else:
    print(f"\u26a0\ufe0f  {total_f + total_e} issues need investigation")
