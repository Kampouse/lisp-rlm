; probe v3: bisect the sibling-call corruption inside the NEAR contract env.
; Each qN returns its result; the scenario asserts with "expect".
; q1 reference walk; q2 exact probe-P2; q3 routes through a user fn;
; q4 drops hosts (literal lists); q5 drops let* as well.
(define (fail m) (near/log m) (near/abort m))
(define (g k d) (default (near/json_get_str k) d))

; L: reference recursion, one cdr per step (string-typed)
(define (lenn xs) (if (nil? xs) "0" (u128/add "1" (lenn (cdr xs)))))

; q1: reference walk, no sibling calls
(define (q1)
  (near/return (lenn (str-split (g "xs" "") ","))))

; q2: EXACT probe-P2 — sibling len calls, hosts via g, let* str-split
(define (q2)
  (let* ((xs (str-split (g "xs" "") ","))
         (ys (str-split (g "ys" "") ",")))
    (near/return (u128/add (to-string (lenn (cdr xs)))
                           (to-string (lenn (cdr ys)))))))

; q3: same but args route through a USER fn (pick2)
(define (pick2 a b) (u128/add a b))
(define (q3)
  (let* ((xs (str-split (g "xs" "") ","))
         (ys (str-split (g "ys" "") ",")))
    (near/return (pick2 (to-string (lenn (cdr xs)))
                        (to-string (lenn (cdr ys)))))))

; q4: hosts removed (literal lists through let*)
(define (q4)
  (let* ((xs (str-split "a,b,c" ","))
         (ys (str-split "d,e,f" ",")))
    (near/return (u128/add (to-string (lenn (cdr xs)))
                           (to-string (lenn (cdr ys)))))))

; q5: let* removed too (inline list literals)
(define (q5)
  (near/return (u128/add (to-string (lenn (cdr (list "a" "b" "c"))))
                         (to-string (lenn (cdr (list "d" "e" "f")))))))

; q6: OLD accruef shape — sibling BUILTIN cdr calls as direct args of a
; recursive USER call, no user/builtin name collision anywhere.
; 3-element gs must count 3.
(define (count gs ls)
  (if (nil? gs) "0"
      (u128/add "1" (count (cdr gs) (cdr ls)))))
(define (q6)
  (near/return (count (str-split (g "gs" "") ",")
                      (str-split (g "ls" "") ","))))

; q7: three sibling BUILTIN calls (cdr cdr car) as direct args of one
; USER call — returns the car, so "a" proves all three survived.
(define (pick3 a b c) c)
(define (q7)
  (let* ((xs (str-split (g "xs" "") ","))
         (ys (str-split (g "ys" "") ",")))
    (near/return (pick3 (cdr xs) (cdr ys) (car xs)))))

; q8: SHADOW test — user fn named like a builtin (t14 class, near mode).
; User lenx shadows nothing; THIS one is named len and must shadow the
; builtin: (len "abc") under shadow rules calls the USER fn (nil? "abc"
; is a type error => trap) or the builtin (str len = 3). We return a
; marker to see which one ran.
; q8 (compile-rejection witness, git history only): (define (len x) …)
; + (near/return (len "abc")) fails with "str ≠ int" REPORTED AT LINE 13
; (q1, an unrelated export) — the checker resolves the shadowed `len` to
; the BUILTIN (str→int) and the error position is the first near/return
; in the file, not the offending call.

; q9 (compile-rejection witness, kept in git history only): passing the
; STR (car gs) into the num ck param is rejected — "type mismatch:
; num ≠ str". One bad def poisons the whole file, so it can't live here.
; q10: CORRECT typing — ck num end-to-end ((str->num (car gs)) in the
; recursion). Must compile AND return 7 for gs=8,12,28 l=2^62.
(define (df a b) (u128/muldiv a b "18446744073709551616"))
(define (accn gs ls ck)
  (if (nil? gs) "0"
      (begin
        (if (< (str->num (car gs)) ck) (fail "ERR_ORDER") "ok")
        (u128/add (df (to-string (- (str->num (car gs)) ck)) (car ls))
                  (accn (cdr gs) (cdr ls) (str->num (car gs)))))))
(define (q10)
  (near/return (accn (str-split (g "gs" "") ",")
                     (str-split (g "ls" "") ",")
                     (str->num "0"))))

; q11: accn without begin/guard — pure if, direct-call siblings kept,
; num-arith args kept ((str->num …), (- …)). 3 elems, l=2^62: still 7.
(define (acc11 gs ls ck)
  (if (nil? gs) "0"
      (u128/add (df (to-string (- (str->num (car gs)) ck)) (car ls))
                (acc11 (cdr gs) (cdr ls) (str->num (car gs))))))
(define (q11)
  (near/return (acc11 (str-split (g "gs" "") ",")
                      (str-split (g "ls" "") ",")
                      (str->num "0"))))

; q12: no num-arith at all — terms are (df (car gs) (car ls)) on the raw
; strings, recursion passes (car gs) as the (unused-as-num) ck.
; l = 2^61: terms 4,2,8 => 14 if all survive.
(define (acc12 gs ls)
  (if (nil? gs) "0"
      (u128/add (df (car gs) (car ls))
                (acc12 (cdr gs) (cdr ls)))))
(define (q12)
  (near/return (acc12 (str-split (g "gs" "") ",")
                      (str-split (g "ls" "") ","))))

; q13-q16: runtime value probes — is the zero term int_to_str, df, or
; their combination? All standalone, no recursion.
(define (q13)
  (near/return (to-string (- (str->num "28") (str->num "12")))))
(define (q14)
  (near/return (df (to-string (- (str->num "28") (str->num "12")))
                   "2305843009213693952")))
(define (q15)
  (near/return (df "16" "2305843009213693952")))
(define (q16)
  (near/return (to-string 16)))

; q17: hand-unrolled 3-frame fold, NO recursion — same values, same
; shapes as q11 on gs=8,12,28 l=2^62. 7 if composition is innocent.
(define (q17)
  (let* ((cval (u128/add (df (to-string (- (str->num "28") (str->num "12")))
                              "4611686018427387904")
                         "0"))
         (bval (u128/add (df (to-string (- (str->num "12") (str->num "8")))
                              "4611686018427387904")
                         cval))
         (aval (u128/add (df (to-string (- (str->num "8") (str->num "0")))
                              "4611686018427387904")
                         bval)))
    (near/return aval)))

; q18: same, but every intermediate sunk to storage for the scenario to
; pin: c0=4 c1=4 b1=5, return 7.
(define (q18)
  (let* ((c0 (df (to-string (- (str->num "28") (str->num "12")))
                 "4611686018427387904"))
         (c1 (u128/add c0 "0"))
         (b1 (u128/add (df (to-string (- (str->num "12") (str->num "8")))
                            "4611686018427387904")
                       c1))
         (a1 (u128/add (df (to-string (- (str->num "8") (str->num "0")))
                            "4611686018427387904")
                       b1)))
    (begin
      (near/storage_set "p:c0" c0)
      (near/storage_set "p:c1" c1)
      (near/storage_set "p:b1" b1)
      (near/return a1))))

; q19: DEPTH-KEYED instrumented fold — sinks ck, car gs, and the df term
; per frame to storage (p:<d>c / p:<d>g / p:<d>t) so the scenario can
; read what each recursion frame actually saw.
(define (acc19 gs ls ck d)
  (if (nil? gs) "0"
      (begin
        (near/storage_set (str-cat "p:" (str-cat d "c")) (to-string ck))
        (near/storage_set (str-cat "p:" (str-cat d "g")) (car gs))
        (near/storage_set (str-cat "p:" (str-cat d "t"))
          (df (to-string (- (str->num (car gs)) ck)) (car ls)))
        (acc19 (cdr gs) (cdr ls) (str->num (car gs))
               (to-string (+ (str->num d) 1))))))
(define (q19)
  (near/return (acc19 (str-split (g "gs" "") ",")
                      (str-split (g "ls" "") ",")
                      (str->num "0") "0")))

; q20: q11 EXACTLY, plus per-frame sinks of BOTH add operands:
; p:<d>a = df term (left), p:<d>r = recursive result (right).
(define (acc20 gs ls ck d)
  (if (nil? gs) "0"
      (u128/add
        (near/storage_set (str-cat "p:" (str-cat d "a"))
          (df (to-string (- (str->num (car gs)) ck)) (car ls)))
        (acc20 (cdr gs) (cdr ls) (str->num (car gs))
               (to-string (+ (str->num d) 1))))))
(define (q20)
  (near/return (acc20 (str-split (g "gs" "") ",")
                      (str-split (g "ls" "") ",")
                      (str->num "0") "0")))


; q21: q11 with the df term let*-bound before the add (house workaround).
; Must return 7. Diff acc11 vs acc21 WATs = the fix recipe.
(define (acc21 gs ls ck)
  (if (nil? gs) "0"
      (let* ((t (df (to-string (- (str->num (car gs)) ck)) (car ls)))
             (r (acc21 (cdr gs) (cdr ls) (str->num (car gs)))))
        (u128/add t r))))
(define (q21)
  (near/return (acc21 (str-split (g "gs" "") ",")
                      (str-split (g "ls" "") ",")
                      (str->num "0"))))


; m1-m6: add-operand matrix. L2=2^62: df("16",L2)=4, df("8",L2)=2.
(define (m1) (near/return (u128/add (df "16" "4611686018427387904") "0")))
(define (m2) (near/return (u128/add "4" "0")))
(define (m3) (near/return (u128/add (df "16" "4611686018427387904")
                                    (df "16" "4611686018427387904"))))
(define (m4) (near/return (u128/add "1" (u128/add (df "16" "4611686018427387904") "0"))))
(define (m5) (near/return (u128/add (df "8" "4611686018427387904") "2")))
(define (m6)
  (let* ((a (df "16" "4611686018427387904")))
    (near/return (u128/add a "0"))))


; acc13: 3-param ck-threading via str->num, NO arith in the term.
; gs=8,12,28 l=2^62 -> terms df(8),df(12),df(28)=2,3,7 sum 12.
(define (acc13 gs ls ck)
  (if (nil? gs) "0"
      (u128/add (df (car gs) (car ls))
                (acc13 (cdr gs) (cdr ls) (str->num (car gs))))))
(define (q13b)
  (near/return (acc13 (str-split (g "gs" "") ",")
                      (str-split (g "ls" "") ",")
                      (str->num "0"))))
; acc14: arith term ((- (str->num (car gs)) "0")), 2-param, no ck threading.
; Same inputs: deltas 8,12,28 -> 2,3,7 sum 12.
(define (acc14 gs ls)
  (if (nil? gs) "0"
      (u128/add (df (to-string (- (str->num (car gs)) (str->num "0"))) (car ls))
                (acc14 (cdr gs) (cdr ls)))))
(define (q14b)
  (near/return (acc14 (str-split (g "gs" "") ",")
                      (str-split (g "ls" "") ","))))


; q22: acc13 + per-frame sinks of BOTH the ck arg expr result and the
; (car gs) it came from: p:<d>k = (str->num (car gs)) pre-call,
; p:<d>c = ck as received next frame.
(define (acc22 gs ls ck d)
  (if (nil? gs) "0"
      (begin
        (near/storage_set (str-cat "p:" (str-cat d "k"))
          (to-string (str->num (car gs))))
        (u128/add (df (car gs) (car ls))
                  (acc22 (cdr gs) (cdr ls) (str->num (car gs))
                         (to-string (+ (str->num d) 1)))))))
(define (q22)
  (near/return (acc22 (str-split (g "gs" "") ",")
                      (str-split (g "ls" "") ",")
                      (str->num "0") "0")))


; q23: EXACT acc11 shape + begin/sink of (to-string ck) per frame.
; q24: same + sink of the sub result (to-string (- (str->num (car gs)) ck)).
(define (acc23 gs ls ck d)
  (if (nil? gs) "0"
      (begin
        (near/storage_set (str-cat "p:" (str-cat d "x")) (to-string ck))
        (u128/add (df (to-string (- (str->num (car gs)) ck)) (car ls))
                  (acc23 (cdr gs) (cdr ls) (str->num (car gs))
                         (to-string (+ (str->num d) 1)))))))
(define (q23)
  (near/return (acc23 (str-split (g "gs" "") ",")
                      (str-split (g "ls" "") ",")
                      (str->num "0") "0")))


; q25: marshal-vs-eval splitter. arg3 is wrapped: sink3 evaluates the
; EXACT arg expr, sinks it, returns it unchanged.
(define (sink3 v) (begin (near/storage_set "p:k3" (to-string v)) v))
(define (acc25 gs ls ck)
  (if (nil? gs) "0"
      (u128/add (df (to-string (- (str->num (car gs)) ck)) (car ls))
                (acc25 (cdr gs) (cdr ls)
                       (sink3 (str->num (car gs)))))))
(define (q25)
  (near/return (acc25 (str-split (g "gs" "") ",")
                      (str-split (g "ls" "") ",")
                      (str->num "0"))))

(export "q1" q1)
(export "q2" q2)
(export "q3" q3)
(export "q4" q4)
(export "q5" q5)
(export "q6" q6)
(export "q7" q7)
(export "q10" q10)
(export "q11" q11)
(export "q12" q12)
(export "q13" q13)
(export "q14" q14)
(export "q15" q15)
(export "q16" q16)
(export "q17" q17)
(export "q18" q18)
(export "q19" q19)
(export "q20" q20)
(export "q21" q21)
(export "m1" m1)
(export "m2" m2)
(export "m3" m3)
(export "m4" m4)
(export "m5" m5)
(export "m6" m6)
(export "q13b" q13b)
(export "q14b" q14b)
(export "q22" q22)
(export "q23" q23)
(export "q25" q25)
