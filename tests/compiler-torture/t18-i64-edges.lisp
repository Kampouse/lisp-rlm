;; T18 — i64 boundary edges: MIN/MAX literals, checked overflow, truncation
;;
;; Ground rules: (2) constants from python3 (i64 range ±9223372036854775808);
;; (3) overflow must be a HARD ERROR (t18b), never a silent wrap.
;; Semantics pinned:
;;  - the literal -9223372036854775808 (i64::MIN) parses and prints exactly
;;  - subtraction reaching i64::MIN stays exact; crossing it errors (t18b
;;    covers the + side; (- i64::MIN 1) errors the same way)
;;  - u128/from-i64 of a negative i64 produces the SIGNED decimal string
;;    ("-9223372036854775808") — pinned as actual, matches u128-as-string
;;    representation where the sign is carried in the string, not the value

(println 9223372036854775807)                      ; i64::MAX, prints exact
(println (- 0 (- 0 9223372036854775807)))          ; i64::MAX via negation
(println -9223372036854775808)                     ; i64::MIN literal
(println (- (- 0 9223372036854775807) 1))          ; i64::MIN via arithmetic
(println (+ 9223372036854775807 0))                ; MAX + 0 = MAX, no overflow
(println (- -9223372036854775808 -1))              ; MIN - (-1) = MIN+1, exact
                                                   ; (* 4611686018427387904 2) = 2^63 →
                                                   ; overflow: error class pinned in t18b
(println (to-string 9223372036854775807))          ; "9223372036854775807"
(println (to-string -9223372036854775808))         ; "-9223372036854775808"
(println (u128/from-i64 9223372036854775807))      ; "9223372036854775807"
(println (u128/from-i64 -9223372036854775808))     ; "-9223372036854775808" (signed repr — pinned)
(println (u128/to-i64 "9223372036854775807"))      ; 9223372036854775807 — exact fit
(println (mod 7 3))                                ; 1
(println (mod -7 3))                                ; 2 — euclidean: -7 = -3*3 + 2
(println (/ -9223372036854775808 1))               ; MIN / 1 = MIN, exact
