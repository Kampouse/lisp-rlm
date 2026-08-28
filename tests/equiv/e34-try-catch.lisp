;; e34 — try/catch (round 4): guarded-fallible-op lowering on wasm.
;; Classes: arity, arith type misuse, u128 parse/underflow/div0, int
;; div/mod-by-zero, mul/add overflow, nesting, handler result flow.
;; NOTE: the catch variable binds a message string whose TEXT differs per
;; surface (interp = interpreter message, wasm = emitter message) — probes
;; must not println e itself, only handler results.
(define (f2 a b) (+ a b))
(define (main)
  ;; arity (static fold on wasm)
  (println (try (f2 1) (catch e "err-missing")))
  (println (try (f2 1 2 3) (catch e "err-extra")))
  ;; arith type misuse (literal non-nums)
  (println (try (+ "a" 1) (catch e "err-type")))
  (println (try (+ nil 5) (catch e "err-type-2")))
  ;; u128 edges
  (println (try (u128/add "abc" "1") (catch e "err-parse")))
  (println (try (u128/sub "1" "2") (catch e "err-under")))
  (println (try (u128/div "1" "0") (catch e "err-div0")))
  ;; u128 COMPARISON ops under try (wasm-fuzz find #5, 2026-08-27):
  ;; lt/gt/eq inlined their parse calls and TRAPPED uncatchably on invalid
  ;; operands while add/sub/mul caught — now all catch.
  (println (try (u128/gt "1" "") (catch e "g-empty")))
  (println (try (u128/lt "" "1") (catch e "l-empty")))
  (println (try (u128/eq "x" "1") (catch e "e-x")))
  ;; int div/mod by zero (runtime guards)
  (println (try (/ 10 0) (catch e "err-div0")))
  (println (try (mod 10 0) (catch e "err-mod0")))
  ;; overflow — runtime mul + compile-fold add
  (println (try (* 3037000500 3037000500) (catch e "err-mul-ovf")))
  (println (try (+ 576460752303423487 576460752303423487) (catch e "err-add-ovf")))
  ;; non-error path: try passes the body value through
  (println (try (+ 1 2) (catch e "no")))
  (println (try "just-a-string" (catch e "no")))
  ;; nested: inner catches, outer unaffected
  (println (try (+ 1 (try (/ 5 0) (catch e2 7))) (catch e "no")))
  ;; handler can use the surrounding scope
  (let ((tag "ctx:"))
    (println (try (u128/mul "not-num" "1") (catch e (str-cat tag "caught"))))
    (println (str-cat (str-cat tag "ok") "!")))
  ;; catch var shadows outer binding inside handler only (e's value is
  ;; surface-specific — never print it; check the handler value + that the
  ;; outer binding survives)
  (let ((e "outer"))
    (println (try (/ 1 0) (catch e "shadow-h")))
    (println e)))
