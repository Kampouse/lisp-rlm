;; e37 — KNOWN DIVERGENCE (wasm-fuzz find #7, 2026-08-27, UNFIXED)
;;
;; Runtime binary `-` with a COMPUTED left operand and negative right
;; operand flips the result sign near the 2^59 boundary:
;;   X = (mod -576460752303423488 1152921504606846975)  ; = 2^59-1, computed
;;   (- X -1)  → interp: 576460752303423488 (2^59, correct)
;;             → wasm:  -576460752303423487 (-(X-1), SIGN FLIPPED)
;;
;; Triggers: computed LEFT + literal/computed negative RIGHT.
;; Literal-left cases const-fold and are correct. checked_sub itself is
;; standard; the flip is elsewhere in the emitted sequence (mod helper
;; interaction suspected — see /tmp/bs.wasm disassembly from the find).
;; This is SILENT VALUE CORRUPTION on the shipped surface — keep this
;; probe red until fixed.
(define (main)
  (println (- (mod -576460752303423488 1152921504606846975) -1))
  (println (- (mod -576460752303423488 1152921504606846975) (str-index-of "0" "a:b:c")))
  ;; controls — these are correct on both surfaces:
  (println (- 576460752303423487 -1))
  (println (- 576460752303423487 (str-index-of "0" "a:b:c")))
  (println (+ 576460752303423487 1))
)
