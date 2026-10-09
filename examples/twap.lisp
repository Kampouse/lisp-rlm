;;;
;;; twap — TWAP execution scheduler (docs/twap-interval-design.md, v1)
;;;
;;; Lazy contract cursor + permissionless tick — no keepers, no promise
;;; chain. Any caller executes the next due slice and earns the executor
;;; fee; worst case is schedule slippage, never stuck funds.
;;;
;;; Clock: block HEIGHT (doc §3.1) — monotonic, not validator-nudgeable.
;;; Users think minutes (~60 heights); all stored/deadline math is u128
;;; decimal strings (house rule: contract math never leaves the u128
;;; string domain).
;;;
;;; Methods:
;;;   create     payable — escrow total_in (attached yocto), store order
;;;   tick       permissionless — execute ONE slice:
;;;                due      → swap via one near/call-await to the pool,
;;;                           credited by the on_slice receipt callback
;;;                expired  → mark unfilled, accrue slice_in refund
;;;                (v1 deviation: one slice per tick, not K=4 — doc §6.3
;;;                 leaves K open; one-per-tick keeps gas flat and every
;;;                 call earns, which is the stronger executor incentive)
;;;   on_slice   receipt callback — fee split, cursor advance, credit
;;;   cancel     seller-only before first slice due → full refund
;;;   finalize   once all slices filled/expired → refund expired slice_in
;;;              to seller, min_avg_out verdict in status
;;;   get_order  view
;;;
;;; Money flow per filled slice:  slice_in --pool.swap--> slice_out
;;;   executor fee  slice_out * fee_bps / 10000   (fee_bps = min of
;;;                market default and the seller's cap, set at create)
;;;   protocol cut  slice_out * PROTO_BPS / 10000 → BURN_ADDR
;;;   remainder     credited to the order's recipient (accumulator key)
;;;
;;; LANDMINE COMPLIANCE (pool_v3 house pattern):
;;;   no lambdas/closures; u128 operands let-bound before the next u128
;;;   call; contract math in u128 strings; u128/lt|gt|eq usable directly
;;;   as if-conditions; values let-bound before near/return; str→str
;;;   storage only; statement-branch ifs use the (if c stmt 0) form.

;; ── policy constants ──
(define FEE_BPS   "100")     ;; market default executor fee, 1%
(define PROTO_BPS "50")      ;; protocol cut → burn, 0.5% ($CROSS thesis)
(define BP_TOTAL  "10000")
(define BURN_ADDR "burn.near")
(define TGAS 30000000000000) ;; promise gas (Num literal — host arg)
(define MAX_SLICES "1000")

;; ── storage helpers (str → str) ──
(define (sget k d) (default (near/storage_get k) d))
(define (sput k v) (near/storage_set k v))

(define (fail m)
  (near/log m)
  (near/abort m))

;; storage keys — "o:" + id + ":" + field
(define (k-of id f) (str-cat "o:" (str-cat id (str-cat ":" f))))

(define (g id f d) (sget (k-of id f) d))
(define (p id f v) (sput (k-of id f) v))

;; ── create ──
;; args: pool, token_out, recipient, n_slices, interval_h, start_h,
;;       grace_h, min_out_per_slice, min_avg_out, fee_bps_cap
;; funds: attached yocto = total_in (escrowed by this contract)
(define (create)
  (let ((pool (default (near/json_get_str "pool") ""))
        (tok (default (near/json_get_str "token_out") ""))
        (rcp (default (near/json_get_str "recipient") ""))
        (ns  (default (near/json_get_str "n_slices") "0"))
        (iv  (default (near/json_get_str "interval_h") "0"))
        (sh  (default (near/json_get_str "start_h") "0"))
        (gh  (default (near/json_get_str "grace_h") "0"))
        (mo  (default (near/json_get_str "min_out_per_slice") "0"))
        (ma  (default (near/json_get_str "min_avg_out") "0"))
        (fc  (default (near/json_get_str "fee_bps_cap") FEE_BPS))
        (dep (near/attached_deposit_u128))
        (h   (near/block_height)))
    (if (u128/eq dep "0")
        (fail "ERR_ZERO")
        (if (= pool "")
            (fail "ERR_ARGS")
            (if (u128/eq ns "0")
                (fail "ERR_ARGS")
                (if (u128/lt MAX_SLICES ns)
                    (fail "ERR_ARGS")
                    (if (u128/eq iv "0")
                        (fail "ERR_ARGS")
                        (if (u128/eq gh "0")
                            (fail "ERR_ARGS")
                            ;; start_h defaults to now + interval
                            (let ((sh2 (if (u128/eq sh "0")
                                           (u128/add (u128/from-i64 h) iv)
                                           sh)))
                              (let ((id (sget "seq" "0")))
                                (let ((id2 (u128/add id "1")))
                                  (sput "seq" id2)
                                  (p id2 "seller" (near/predecessor_account_id))
                                  (p id2 "pool" pool)
                                  (p id2 "token" tok)
                                  (p id2 "rcp" rcp)
                                  (p id2 "ns" ns)
                                  (p id2 "iv" iv)
                                  (p id2 "sh" sh2)
                                  (p id2 "gh" gh)
                                  (p id2 "mo" mo)
                                  (p id2 "ma" ma)
                                  (p id2 "fc" fc)
                                  (p id2 "nx" "0")
                                  (p id2 "fo" "0")
                                  (p id2 "rx" "0")
                                  (p id2 "xp" "0")
                                  (p id2 "st" "open")
                                  ;; slice_in = total_in / n_slices; the
                                  ;; remainder rides the last slice
                                  (let ((si (u128/div dep ns)))
                                    (let ((used (u128/mul si ns)))
                                      (let ((le (u128/sub dep used)))
                                        (p id2 "si" si)
                                        (p id2 "le" le)
                                        (near/return id2)))))))))))))))

;; ── tick — the permissionless engine ──
;; One call advances the order by ONE slice (swap or expiry).
(define (tick)
  (let ((id (default (near/json_get_str "order_id") ""))
        (h  (near/block_height))
        (ex (near/predecessor_account_id)))
    (let ((seller (g id "seller" "")))
      (if (= seller "")
          (fail "ERR_NO_ORDER")
          (let ((nx (g id "nx" "0"))
                (ns (g id "ns" "0"))
                (sh (g id "sh" "0"))
                (iv (g id "iv" "0"))
                (gh (g id "gh" "0")))
            (if (u128/eq nx ns)
                (fail "ERR_DONE")
                ;; due_h = sh + nx*iv ; exp_h = due_h + gh
                (let ((off (u128/mul nx iv)))
                  (let ((due (u128/add sh off)))
                    (let ((exp (u128/add due gh)))
                      (let ((hu (u128/from-i64 h)))
                        (if (u128/lt hu due)
                            (fail "ERR_NOT_DUE")
                            (if (u128/lt exp hu)
                                (tick-expired id nx)
                                (tick-swap id nx ex)))))))))))))

(define (tick-expired id nx)
  ;; slice past grace: unfilled, its input becomes refundable
  (let ((si (g id "si" "0"))
        (ns (g id "ns" "0"))
        (xp (g id "xp" "0")))
    (let ((nx1 (u128/add nx "1")))
      (let ((xp1 (u128/add xp "1")))
        (p id "nx" nx1)
        (p id "xp" xp1)
        ;; last slice carries the remainder
        (if (u128/eq nx1 ns)
            (let ((le (g id "le" "0")))
              (let ((si2 (u128/add si le)))
                (let ((rx (g id "rx" "0")))
                  (let ((rx2 (u128/add rx si2)))
                    (p id "rx" rx2)
                    (near/return "expired")))))
            (let ((rx (g id "rx" "0")))
              (let ((rx2 (u128/add rx si)))
                (p id "rx" rx2)
                (near/return "expired"))))))))

(define (tick-swap id nx ex)
  ;; due slice: remember the executor (receipt callbacks see THIS
  ;; contract as predecessor, so the fee payee is captured here), then
  ;; fire the single swap promise. The await consumed here is exactly
  ;; one per defn — the single-use gate's happy path.
  (sput (str-cat id "|ex") ex)
  (let ((pool (g id "pool" ""))
        (ns (g id "ns" "0"))
        (si (g id "si" "0")))
    ;; last slice: attach the escrow remainder too
    (let ((nx1 (u128/add nx "1")))
      (let ((amt (if (u128/eq nx1 ns)
                     (let ((le (g id "le" "0")))
                       (u128/add si le))
                     si)))
        (let ((args (str-cat "{\"order\":\"" id "\",\"k\":\"" nx "\"}")))
          (near/call-await pool "swap" args TGAS "on_slice" TGAS
                           (str-cat "{\"order\":\"" id "\",\"k\":\"" nx
                                    "\",\"amt\":\"" amt "\"}"))
          (near/return "queued"))))))

;; ── on_slice — receipt callback ──
;; cb_args (our input): order, k, amt. Pool returned slice_out.
(define (on-slice)
  (let ((id (default (near/json_get_str "order") ""))
        (k  (default (near/json_get_str "k") "0"))
        (out (near/promise_result 0)))
    (let ((mo (g id "mo" "0")))
      (if (= out "")
          ;; swap receipt failed → slice stays due for a retry tick
          (begin (near/log "SLICE_RETRY") (near/return "retry"))
          (if (u128/lt out mo)
              ;; slippage guard → stays due, retry until grace (doc §3.4,
              ;; strict retry; no auto-narrow)
              (begin (near/log "SLICE_RETRY") (near/return "retry"))
              (let ((fc (g id "fc" FEE_BPS))
                    (nx (g id "nx" "0")))
                ;; fee_bps = min(market default, seller cap)
                (let ((fb (if (u128/lt FEE_BPS fc) FEE_BPS fc)))
                  (let ((fnum (u128/mul out fb)))
                    (let ((fee (u128/div fnum BP_TOTAL)))
                      (let ((pnum (u128/mul out PROTO_BPS)))
                        (let ((cut (u128/div pnum BP_TOTAL)))
                          (let ((net (u128/sub out fee)))
                            (let ((net2 (u128/sub net cut)))
                              (let ((fo (g id "fo" "0")))
                                (let ((fo2 (u128/add fo net2)))
                                  (let ((nx1 (u128/add nx "1")))
                                    (p id "fo" fo2)
                                    (p id "nx" nx1)
                                    (if (u128/eq fee "0")
                                        0
                                        (near/transfer_u128
                                         (sget (str-cat id "|ex") "")
                                         fee))
                                    (if (u128/eq cut "0")
                                        0
                                        (near/transfer_u128 BURN_ADDR cut))
                                    (near/log (str-cat "filled:" k))
                                    (near/return net2)))))))))))))))))

;; ── cancel — seller only, before the first slice is due ──
(define (cancel)
  (let ((id (default (near/json_get_str "order_id") ""))
        (who (near/predecessor_account_id))
        (h (near/block_height)))
    (let ((seller (g id "seller" "")))
      (if (= seller "")
          (fail "ERR_NO_ORDER")
          (if (not (= seller who))
              (fail "ERR_PERM")
              (let ((sh (g id "sh" "0"))
                    (nx (g id "nx" "0")))
                (let ((hu (u128/from-i64 h)))
                  (if (u128/lt hu sh)
                      (if (u128/eq nx "0")
                          ;; total = si*ns + le — recompute from parts
                          (let ((si (g id "si" "0"))
                                (ns (g id "ns" "0")))
                            (let ((used (u128/mul si ns)))
                              (let ((le (g id "le" "0")))
                                (let ((tot (u128/add used le)))
                                  (p id "st" "cancelled")
                                  (near/transfer_u128 seller tot)
                                  (near/return "cancelled")))))
                          (fail "ERR_LATE"))
                      (fail "ERR_LATE")))))))))

;; ── finalize — all slices filled or expired ──
;; refunds accrued expired slice_in to the seller; status carries the
;; min_avg_out verdict (filled_ok | avg_missed | avg_skip).
;; Callable once every slice has been consumed (nx == ns) — expired
;; slices carry their refund in rx.
(define (finalize)
  (let ((id (default (near/json_get_str "order_id") "")))
    (let ((seller (g id "seller" "")))
      (if (= seller "")
          (fail "ERR_NO_ORDER")
          (let ((nx (g id "nx" "0"))
                (ns (g id "ns" "0")))
            (if (u128/eq nx ns)
                (let ((fo (g id "fo" "0"))
                      (rx (g id "rx" "0"))
                      (xp (g id "xp" "0"))
                      (ma (g id "ma" "0")))
                  (let ((filled (u128/sub nx xp)))
                    (let ((verdict
                           (if (u128/eq filled "0")
                               "avg_skip"
                               (if (u128/lt fo (u128/mul ma filled))
                                   "avg_missed"
                                   "filled_ok"))))
                      (p id "st" verdict)
                      (if (u128/eq rx "0")
                          0
                          (near/transfer_u128 seller rx))
                      (near/return verdict))))
                (fail "ERR_OPEN")))))))

;; ── view ──
(define (get-order)
  (let ((id (default (near/json_get_str "order_id") "")))
    (let ((seller (g id "seller" "")))
      (if (= seller "")
          (near/return "null")
          (let ((j1 (json-set "{}" "seller" (str-cat "\"" seller "\""))))
            (let ((j2 (json-set j1 "pool" (str-cat "\"" (g id "pool" "") "\""))))
              (let ((j3 (json-set j2 "ns" (str-cat "\"" (g id "ns" "0") "\""))))
                (let ((j4 (json-set j3 "nx" (str-cat "\"" (g id "nx" "0") "\""))))
                  (let ((j5 (json-set j4 "fo" (str-cat "\"" (g id "fo" "0") "\""))))
                    (let ((j6 (json-set j5 "rx" (str-cat "\"" (g id "rx" "0") "\""))))
                      (let ((j7 (json-set j6 "xp" (str-cat "\"" (g id "xp" "0") "\""))))
                        (let ((j8 (json-set j7 "si" (str-cat "\"" (g id "si" "0") "\""))))
                          (let ((j9 (json-set j8 "st" (str-cat "\"" (g id "st" "open") "\""))))
                            (near/return j9))))))))))))))

(define (on_slice) (on-slice))
(define (get_order) (get-order))

(export "create" create)
(export "tick" tick)
(export "on_slice" on_slice)
(export "cancel" cancel)
(export "finalize" finalize)
(export "get_order" get_order #t)
