;; error: the CLMM-v5 corruption shape — a misplaced close-paren turns the
;; if into a FOUR-positional-form if: (if C then else extra). Since the
;; 2026-10-15 arity cap this is a HARD compile error (checker + wasm
;; emitter + bytecode), never a silent extra-arg drop. Message contains
;; "if" + "positional" ("misplaced ')'").
;;
;; The corruption's origin: the else-begin below was orphaned INSIDE the
;; then-arm's let* body by a missing closer —
;;   (if C (let* (...) body1 body2) else)  ;; 2-form if, let* body2 —
;; which stays LEGAL (implicit begin; the cap cannot reject it without
;; breaking real programs). This file pins the DETECTABLE form: the
;; extra arm surfaces as a positional 4th. Pre-guard, it compiled green
;; and the extra (up) form silently won (v5 pool swap arc: down-swap ran
;; both arms, returned the up constant, wrote UPPATH after DOWNPATH).
(define (arm v)
  (if (str= (str-slice v 0 1) "-")
      (str-cat "D" v)
      (str-cat "U" v)
      (str-cat "X" v)))
