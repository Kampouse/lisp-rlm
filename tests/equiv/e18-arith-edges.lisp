;; e18 — i64-range literals: PERMANENT representation divergence.
;; Interp payloads are i64; wasm tagged payloads are 61-bit [-2^60, 2^60).
;; Literals beyond ±2^60 hard-error at wasm compile (money-safety refusal),
;; while the interpreter evaluates them. Both-error with different causes —
;; documented, not fixable without a wider tag scheme.
;; (try-catchable arith edges moved to e34.)
(define (main)
  (println (- 0 9223372036854775807))
  (println (try (- (- (- 0 9223372036854775807) 1) 1) (catch e "err-ovf"))))
