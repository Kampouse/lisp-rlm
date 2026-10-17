;; pool-ledger.lisp — Pool_Ledger.thy port (AFP afp-clmm, GREEN Oct 15 2026)
;;
;; The TOKEN side of the pool: where the pool's actual balance lives and
;; who it is owed to. Four buckets, the conservation theorem verbatim:
;;   psum = principal + setaside + unalloc + burnt
;;   psum(prun gs s) = psum s + pded gs        (every token accounted)
;;   burnt(prun gs σp0) ≤ pded gs             (pool_no_vaporize)
;;
;; Ops (pstep, same order as the thy):
;;   PDep m    — principal += m                (deposit, mint m shares upstream)
;;   PSweep m  — unalloc += m                  (collected fees arrive)
;;   PGrow f   — f ≤ unalloc ? unalloc-f/setaside+f : identity   (GUARD)
;;   PWd b f   — b ≤ principal ∧ f ≤ setaside ? move to burnt : identity (GUARD)
;;
;; Provenance is a guard: violations are SOUND NO-OPS (thy: "exactly like
;; WdFail"), so grow/payout on shortfall return ok-noop, not ERR — the
;; spec says identity, the code must be identity.
;;
;; Mock simplification: payouts settle the burnt ledger only (no token
;; contract wired). On mainnet the wd leg is the NEP-141 ft_transfer;
;; bookkeeping (burnt bucket) is identical.

(define (sput k v) (near/storage_set k v))
(define (sget k d) (default (near/storage_get k) d))
(define (fail m) (begin (near/log m) (near/abort m)))
(define (add a b) (u128/add a b))
(define (sub a b) (u128/sub a b))
(define (ult a b) (u128/lt a b))

;; ── PDep m (attach): principal += m ──
(define (pdep)
  (let ((m (near/attached_deposit_u128)))
    (begin
      (sput "principal" (add (sget "principal" "0") m))
      (near/log (str-cat "pdep m=" m))
      (near/return "ok"))))

;; ── PSweep m (attach): unalloc += m (collected fees arrive) ──
(define (psweep)
  (let ((m (near/attached_deposit_u128)))
    (begin
      (sput "unalloc" (add (sget "unalloc" "0") m))
      (near/log (str-cat "psweep m=" m))
      (near/return "ok"))))

;; ── PGrow f: label f unalloc → setaside. Guard f ≤ unalloc, else NO-OP ──
(define (pgrow)
  (let* ((f (default (near/json_get_str "f") "0"))
         (ua (sget "unalloc" "0")))
    (if (ult ua f)
        (begin (near/log "noop-overflow") (near/return "noop"))
        (begin
          (sput "unalloc" (sub ua f))
          (sput "setaside" (add (sget "setaside" "0") f))
          (near/log (str-cat "pgrow f=" f))
          (near/return "ok")))))

;; ── PWd b f: pay b principal + f fees. Guard both buckets, else NO-OP ──
(define (pwd)
  (let* ((b (default (near/json_get_str "b") "0"))
         (f (default (near/json_get_str "f") "0"))
         (p (sget "principal" "0"))
         (sa (sget "setaside" "0")))
    (if (ult p b)
        (begin (near/log "noop-principal") (near/return "noop"))
        (if (ult sa f)
            (begin (near/log "noop-setaside") (near/return "noop"))
            (begin
              (sput "principal" (sub p b))
              (sput "setaside" (sub sa f))
              (sput "burnt" (add (sget "burnt" "0") (add b f)))
              (near/log (str-cat "pwd b=" b " f=" f))
              (near/return "ok"))))))

;; ── views: psum and the four buckets ──
(define (get-psum)
  (let* ((p (sget "principal" "0"))
         (sa (sget "setaside" "0"))
         (ua (sget "unalloc" "0"))
         (bu (sget "burnt" "0")))
    (near/return (add p (add sa (add ua bu))))))

(define (get-principal) (near/return (sget "principal" "0")))
(define (get-setaside)  (near/return (sget "setaside" "0")))
(define (get-unalloc)   (near/return (sget "unalloc" "0")))
(define (get-burnt)     (near/return (sget "burnt" "0")))

(export "pdep" pdep)
(export "psweep" psweep)
(export "pgrow" pgrow)
(export "pwd" pwd)
(export "get-psum" get-psum)
(export "get-principal" get-principal)
(export "get-setaside" get-setaside)
(export "get-unalloc" get-unalloc)
(export "get-burnt" get-burnt)
