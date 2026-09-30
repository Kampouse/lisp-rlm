;; e38 — str-contains with DYNAMIC needles (the 2026-09-30 emitter fix:
;; literal needles keep the compile-time fast path; any other expression
;; reuses the dynamic scan via str-index-of-dyn >= 0, tagged BOOL for
;; interp parity). Cases: found / not-found / both-built-at-runtime /
;; empty needle (always-contains, interp parity) / needle from substring.
(define (dyn-tests)
  (list (str-contains "hello world" (str-cat "wor" "ld"))
        (str-contains "hello" (str-cat "x" "yz"))
        (str-contains (str-cat "hello " "world") (str-cat "wo" "rld"))
        (str-contains "hello" "")
        (str-contains "banana" (str-substring "banana" 1 2))
        (str-index-of "banana" (str-cat "a" "na"))))
(dyn-tests)
