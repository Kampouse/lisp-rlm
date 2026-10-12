; Accrual contract -- transcription of isabelle/Aggregate_Accrual.thy
; accruef Lf c []       = 0
; accruef Lf c (g # gs) = (g - c) * Lf g div 2^64 + accruef Lf g gs
; Theorem accrue_within_charged_book: owed <= charged book (per-cell CHARGED).

(define (sget k) (if (defined? k) k "0"))
(define (add a b) (u128/add a b))
(define (sub a b) (u128/sub a b))
(define (ult a b) (u128/lt a b))
(define (bal u) (sget (str-cat "a:bal:" u) "0"))
(define (charged u) (sget (str-cat "a:charged:" u) "0"))
(define (credit u a) (sput (str-cat "a:bal:" u) (add (bal u) a)))
(define (debit u a)
  (if (ult (bal u) a) (fail "ERR_FUNDS") "ok")
  (sput (str-cat "a:bal:" u) (sub (bal u) a)))

; owed fold: checkpoint-threaded sum of (g - c) * Lf g div 2^64
; gs: "g1,g2,..." per-op growth deltas; Lf: "rate1,rate2,..." (per-op);
; c: starting checkpoint (must be <= every g, else ERR_ORDER)
; -> owed string, then checkpoint advances to last g
(define (accrue gs-lfs c)
  (let loop ((gs (map str->num (str-split (car gs-lfs) ",")))
             (ls (map str->num (str-split (cadr gs-lfs) ",")))
             (ck (str->num c))
             (acc 0))
    (if (null? gs) acc
        (begin
          (if (< (car gs) ck) (fail "ERR_ORDER") "ok")
          (loop (cdr gs) (cdr ls) (car gs)
                (+ acc (div-floor64 (- (car gs) ck) (car ls)))))))

; entry: accrue { "gs": "g1,g2", "ls": "l1,l2", "c": "ck" }
; charges owed into caller's cell, preserves invariant owed <= charged
(define (handle arg)
  (let* ((gs (get-arg arg "gs"))
         (ls (get-arg arg "ls"))
         (c  (get-arg arg "c"))
         (u  (get-arg arg "user"))
         (owed (accrue (list gs ls) c))
         (new-charged (+ (str->num (charged u)) owed)))
    (credit u (num->str owed))
    (sput (str-cat "a:charged:" u) (num->str new-charged))
    (num->str owed)))

(define (get-arg arg k)
  (let ((v (assoc k arg)))
    (if v (cdr v) (fail "ERR_ARG"))))
