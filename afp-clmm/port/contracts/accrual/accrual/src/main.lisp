; Accrual contract -- transcription of isabelle/Aggregate_Accrual.thy
; accruef Lf c []       = 0
; accruef Lf c (g # gs) = (g - c) * Lf g div 2^64 + accruef Lf g gs
; Theorem accrue_within_charged_book: owed <= charged book (per-cell CHARGED).

(define (add a b) (u128/add a b))
(define (sub a b) (u128/sub a b))
(define (ult a b) (u128/lt a b))
(define (sget k) (default (near/storage_get k) "0"))
(define (sput k v) (near/storage_set k v))
(define (fail m) (near/log m) (near/abort m))
(define (bal u) (sget (str-cat "a:bal:" u)))
(define (charged u) (sget (str-cat "a:charged:" u)))
(define (credit u a) (sput (str-cat "a:bal:" u) (add (bal u) a)))
(define (debit u a)
  (if (ult (bal u) a) (fail "ERR_FUNDS") "ok")
  (sput (str-cat "a:bal:" u) (sub (bal u) a)))
; div-floor64 a b = (a * b) div 2^64, truncating (floor on nat).
; Mirrors Isabelle div_floor64: owed side rounds DOWN, fail-closed.
; b = 0 yields 0 (accruef semantics), no error.
(define (div-floor64 a b) (u128/muldiv a b "18446744073709551616"))

; Cons step of the owed fold: accruef Lf c (g # gs) instantiated at the
; head -- owed(g,c) = (g - c) * Lf g div 2^64. The checkpoint a:ck:<u>
; carries the fold's c across calls (monotone; g < c aborts ERR_ORDER,
; exactly accruef's precondition). One exported step = one equation
; instance; the scenario's step sequence IS the list gs.
; One call per let* binding (compiler call-sequence discipline: sibling
; calls in one primitive application clobber -- see probe contract).
(define (accrue_step)
  (let* ((g (default (near/json_get_str "g") "0"))
         (l (default (near/json_get_str "l") "0"))
         (u (near/predecessor_account_id))
         (ck-key (str-cat "a:ck:" u))
         (c (default (near/storage_get ck-key) "0"))
         (lt (ult g c))
         (guard (if lt (fail "ERR_ORDER") "ok"))
         (d (u128/sub g c))
         (term (div-floor64 d l)))
    (begin
      (credit u term)
      (let ((ch (charged u)))
        (let ((ch2 (u128/add ch term)))
          (begin
            (sput (str-cat "a:charged:" u) ch2)
            (sput ck-key g)
            (near/return term)))))))

(export "accrue_step" accrue_step)
