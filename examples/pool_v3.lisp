;;;
;;; pool_v3 — JP's NEAR launchpool, fresh v3 in lisp-rlm
;;;
;;; One pool contract; a pool is a mapping token_id → reserves. Buys pay
;;; attached NEAR in and get tokens out via ft_transfer (zero swap fee).
;;; Sells arrive as ft_transfer_call → ft_on_transfer and are paid out in
;;; NEAR (constant product, zero swap fee), minus a protective tax = the
;;; MAX of:
;;;   - decaying flip tax: FLIP_MAX at t=0 since the account's last buy of
;;;     that token, linear to 0 at FLIP_T (no buy record → MAX bracket)
;;;   - market-cap-ratio tax: current NEAR reserve vs the reserve at the
;;;     account's last buy; tiers <2x:0, 2–5x:1000, 5–10x:2500, ≥10x:4000
;;;     (no record → 4000)
;;; Tax split on sells: DEPTH_SPLIT_BP to pool depth (stays in the NEAR
;;; reserve), the rest to a per-pool seniority pot (only when the pool was
;;; launched with seniority on; otherwise 100% to depth).
;;; Seniority: each buy records weight = WNUM / reserve_after_buy (floor).
;;; claim_seniority pays pot * w / Σw (floor), zeroes the claimant's
;;; weight and decrements Σw — a second claim pays 0 until they buy again.
;;; Launch gate: buys revert ERR_EARLY while block_ts < gate_until
;;; (= launch ts + gate_ms, default GATE_MS). Sells are never gated.
;;; Seeds: launch is payable (attached NEAR = initial reserve); initial
;;; tokens arrive via ft_transfer_call with memo `seed`. Tokens that
;;; arrive BEFORE launch are held in an escrow key and credited at launch.
;;;
;;; SELL-GAP FIX (GAPS.md #1): a sell never strands tokens. If the
;;; computed proceeds are 0 (dust), a min_out is not met, or a max_near
;;; cap consumes only part of the sent amount, the unused tokens go back
;;; to the seller through the NEP-141 keep-string (return value = exact
;;; u128 amount kept) so the token contract refunds the surplus in the
;;; same callback.
;;;
;;; ABI (v2-observed surface + v3 additions, guesses flagged [g]):
;;;   launch(token, seniority 0|1, gate_ms? [g default GATE_MS]) payable
;;;   buy(token, min_out?) payable                       → tokens out [g]
;;;   quote_buy(token, near_in) [g near_in arg] view     → {near_in, tokens_out} | null
;;;   ft_on_transfer NEP-141: msg "" | "seed"
;;;       | {"min_out":"<u128>","max_near":"<u128>"} [g]
;;;         max_near caps NEAR proceeds; the tokens actually consumed are
;;;         kept, the surplus is refunded (partial-keep)
;;;   claim_seniority(token)                             → share paid [g]
;;;   get_pool(token) view          → {"near","tokens"} | null
;;;   get_taxes(token) view [g]     → {flip_bp, mcap_bp, pot, weights_sum} for caller
;;;
;;; LANDMINE COMPLIANCE (GAPS.md):
;;;   - no lambdas/closures; plain defines only
;;;   - every u128 operand let-bound before the next u128 call
;;;     (TEMP_MEM scratch reuse); u128/lt|gt|eq return Bool on both
;;;     surfaces, safe in if-conditions
;;;   - all contract math stays in u128 decimal strings (no 61-bit
;;;     arithmetic, no str->num)
;;;   - values let-bound before near/return (double-eval bug)
;;;   - str→str storage via near/storage_set / near/storage_get only
;;;   - if/cond arms that only branch statements use the (if c stmt 0)
;;;     statement form; value-branching ifs keep matching string arms
;;;

;; ── policy constants (u128 decimal strings unless noted) ──
(define GATE_MS  "300000")   ;; default launch gate, milliseconds
(define FLIP_T_MS "600000")  ;; flip tax decay horizon, milliseconds
(define MS_NS    "1000000")  ;; ms → ns
(define GATE_MS_NS (u128/mul GATE_MS MS_NS))
(define FLIP_T_NS (u128/mul FLIP_T_MS MS_NS))
(define FLIP_MAX  "5000")    ;; 50% at t=0
(define BP_TOTAL  "10000")
(define TIER2 "1000")        ;; 2–5x
(define TIER5 "2500")        ;; 5–10x
(define TIER10 "4000")       ;; ≥10x (and the no-record default)
(define DEPTH_SPLIT "5000")  ;; sell tax share that stays in depth
(define WNUM "1000000000000000000000000") ;; seniority weight numerator
(define TGAS 30000000000000) ;; promise gas (Num literal — host arg)

;; ── tiny storage helpers (str → str) ──
(define (sget k d) (default (near/storage_get k) d))
(define (sput k v) (near/storage_set k v))

;; (fail msg) — log then abort (near/abort drops the message in wasm, so
;; the code is logged first; interp sees Err "near/abort: <msg>")
(define (fail m)
  (near/log m)
  (near/abort m))

;; storage key builders — one str-cat site each (wasm size)
(define (k-pn t) (str-cat "pn:" t))
(define (k-pt t) (str-cat "pt:" t))
(define (k-gt t) (str-cat "gt:" t))
(define (k-es t) (str-cat "es:" t))
(define (k-se t) (str-cat "se:" t))
(define (k-po t) (str-cat "po:" t))
(define (k-ws t) (str-cat "ws:" t))
(define (k-ac p t a) (str-cat p t "|" a))

;; ── AMM: constant product, ZERO swap fee, floor division everywhere ──

(define (calc-buy-out tok dep)
  ;; tokens out for `dep` yoctoNEAR — "0" when the pool does not exist
  (let ((pn (sget (k-pn tok) "0"))
        (pt (sget (k-pt tok) "0")))
    (let ((den (u128/add pn dep)))
      (if (u128/eq den "0")
          "0"
          (let ((num (u128/mul dep pt)))
            (u128/div num den))))))

;; ── taxes ──

(define (flip-bp-of lbt now)
  ;; lbt = account's last-buy ts (ns string); "" (no record) → MAX bracket
  (if (= lbt "")
      FLIP_MAX
      (let ((dt (if (u128/lt now lbt)
                    "0"
                    (let ((d (u128/sub now lbt))) d))))
        (let ((tt (if (u128/lt dt FLIP_T_NS) dt FLIP_T_NS)))
          (let ((left (u128/sub FLIP_T_NS tt)))
            (let ((num (u128/mul FLIP_MAX left)))
              (u128/div num FLIP_T_NS)))))))

(define (mcap-bp-of rab rn)
  ;; rab = near reserve at the account's last buy; "" → 4000
  (if (= rab "")
      TIER10
      (let ((x2 (u128/mul rab "2"))
            (x5 (u128/mul rab "5"))
            (x10 (u128/mul rab "10")))
        (cond ((u128/lt rn x2) "0")
              ((u128/lt rn x5) TIER2)
              ((u128/lt rn x10) TIER5)
              (else TIER10)))))

(define (eff-bp-of tok acct now)
  ;; max(flip, mcap) for this account on this pool, at `now`
  (let ((lbt (sget (k-ac "lb:" tok acct) ""))
        (rab (sget (k-ac "rb:" tok acct) ""))
        (rn  (sget (k-pn tok) "0")))
    (let ((fb (flip-bp-of lbt now))
          (mb (mcap-bp-of rab rn)))
      (if (u128/gt fb mb) fb mb))))

;; ── outbound legs (promise receipts) ──

(define (pay-near to amt)
  (near/transfer_u128 to amt))

(define (send-tokens tok to amt)
  ;; ft_transfer(out) to the buyer — single-shot cross call (near/call is
  ;; the one promise surface whitelisted on BOTH the interp compile list
  ;; and the wasm emitter; batch_* names drift between the two)
  (let ((args (str-cat "{\"receiver_id\":\"" to
                      "\",\"amount\":\"" amt "\",\"memo\":\"\"}")))
    (near/call tok "ft_transfer" args TGAS 1)))

;; ── launch ──

(define (launch-core tok sen gate-ms dep ts)
  ;; sen = "1"|"0", gate-ms in ms, dep = attached yocto, ts ns
  (if (u128/eq dep "0")
      (fail "ERR_ZERO")
      (let ((pn (sget (k-pn tok) "")))
        (if (not (= pn ""))
            (fail "ERR_EXISTS")
            (let ((esc (sget (k-es tok) "0")))
              (let ((gns (u128/mul gate-ms MS_NS)))
                (let ((gt (u128/add ts gns)))
                  (sput (k-pn tok) dep)
                  (sput (k-pt tok) esc)
                  (sput (k-es tok) "0")
                  (sput (k-gt tok) gt)
                  (sput (k-se tok) sen)
                  (sput (k-po tok) "0")
                  (sput (k-ws tok) "0")
                  "1")))))))

;; ── buy ──

(define (buy-core tok acct dep mo ts)
  ;; returns tokens out (string); errors: ERR_ZERO, ERR_NO_POOL,
  ;; ERR_EARLY, ERR_SLIP
  (if (u128/eq dep "0")
      (fail "ERR_ZERO")
      (let ((pn (sget (k-pn tok) "")))
        (if (= pn "")
            (fail "ERR_NO_POOL")
            (let ((gt (sget (k-gt tok) "0")))
              (if (u128/lt ts gt)
                  (fail "ERR_EARLY")
                  (let ((pt (sget (k-pt tok) "0")))
                    (let ((num (u128/mul dep pt))
                          (den (u128/add pn dep)))
                      (let ((out (u128/div num den)))
                        (if (u128/eq out "0")
                            (fail "ERR_ZERO")
                            (if (u128/lt out mo)
                                (fail "ERR_SLIP")
                                (let ((pn2 (u128/add pn dep))
                                      (pt2 (u128/sub pt out)))
                                  (sput (k-pn tok) pn2)
                                  (sput (k-pt tok) pt2)
                                  (sput (k-ac "lb:" tok acct) ts)
                                  (sput (k-ac "rb:" tok acct) pn2)
                                  (let ((wnew (u128/div WNUM pn2))
                                        (wold (sget (k-ac "wt:" tok acct) "0"))
                                        (ws   (sget (k-ws tok) "0")))
                                    (sput (k-ac "wt:" tok acct) wnew)
                                    (let ((wsd (u128/sub ws wold)))
                                      (let ((ws2 (u128/add wsd wnew)))
                                        (sput (k-ws tok) ws2)
                                        (send-tokens tok acct out)
                                        out)))))))))))))))

;; ── sell (ft_on_transfer core) ──

(define (cap-keep pn pt amount gross0 mx)
  ;; tokens actually consumed under a max_near cap (rounded UP toward pool
  ;; safety); no cap (mx=0), un-capped, degenerate, or cap ≥ sent → all
  (if (u128/eq mx "0")
      amount
      (if (u128/gt gross0 mx)
          (let ((den (u128/sub pn mx)))
            (if (u128/eq den "0")
                amount
                (let ((num (u128/mul mx pt)))
                  (let ((d1 (u128/sub den "1")))
                    (let ((adj (u128/add num d1)))
                      (let ((tp (u128/div adj den)))
                        (if (u128/gt tp amount)
                            amount
                            tp)))))))
          amount)))

(define (cap-gross pn pt keep amount gross0)
  ;; actual NEAR proceeds for the (possibly capped) keep amount
  (if (= keep amount)
      gross0
      (let ((num (u128/mul keep pn)))
        (let ((den (u128/add pt keep)))
          (u128/div num den)))))

(define (sell-commit tok pt pn keep payout tax)
  ;; commit a sell: token reserve += keep, near reserve -= payout (the
  ;; tax stays in depth), seniority pot credit when the flag is on
  (let ((pt3 (u128/add pt keep)))
    (sput (k-pt tok) pt3)
    (let ((pn2 (u128/sub pn payout)))
      (sput (k-pn tok) pn2)
      (let ((sen (sget (k-se tok) "0")))
        (if (= sen "1")
            (let ((pot (sget (k-po tok) "0")))
              (let ((pnum (u128/mul tax DEPTH_SPLIT)))
                (let ((ps (u128/div pnum BP_TOTAL)))
                  (let ((pot2 (u128/add pot ps)))
                    (sput (k-po tok) pot2)))))
            0)))))

(define (sell-core tok sender amount msg mo mx now)
  ;; returns the u128 keep-string: tokens the pool keeps. Everything not
  ;; kept is refunded to the seller through the NEP-141 keep-string.
  ;; mo = min NEAR out ("0" = none); mx = max NEAR out ("0" = none).
  (if (u128/eq amount "0")
      (fail "ERR_ZERO")
      (if (= msg "seed")
          ;; seed: tokens for the pool's token reserve (pre-launch → escrow)
          (let ((pn (sget (k-pn tok) "")))
            (if (= pn "")
                (let ((esc (sget (k-es tok) "0")))
                  (let ((e2 (u128/add esc amount)))
                    (sput (k-es tok) e2)
                    amount))
                (let ((pt (sget (k-pt tok) "0")))
                  (let ((p2 (u128/add pt amount)))
                    (sput (k-pt tok) p2)
                    amount))))
          ;; plain sell
          (let ((pn (sget (k-pn tok) "")))
            (if (= pn "")
                (fail "ERR_NO_POOL")
                (let ((pt (sget (k-pt tok) "0")))
                  (let ((rt2 (u128/add pt amount)))
                    (let ((gnum (u128/mul amount pn)))
                      (let ((gross0 (u128/div gnum rt2)))
                        (if (u128/eq gross0 "0")
                            "0" ;; dust: nothing to pay — keep nothing
                            (let ((keep (cap-keep pn pt amount gross0 mx)))
                              (let ((gross (cap-gross pn pt keep amount gross0))
                                    (eff (eff-bp-of tok sender now)))
                                (if (u128/eq gross "0")
                                    "0"
                                    (let ((tnum (u128/mul gross eff)))
                                      (let ((tax (u128/div tnum BP_TOTAL)))
                                        (let ((payout (u128/sub gross tax)))
                                          ;; min-out not met → full refund
                                          ;; via keep-string; no state
                                          ;; change, nothing stranded
                                          (if (u128/lt payout mo)
                                              "0"
                                              (begin
                                                (sell-commit tok pt pn keep payout tax)
                                                (if (u128/eq payout "0")
                                                    0
                                                    (pay-near sender payout))
                                                keep))))))))))))))))))

;; ── seniority ──

(define (claim-core tok acct)
  (let ((sen (sget (k-se tok) "0")))
    (if (not (= sen "1"))
        (fail "ERR_SENIORITY")
        (let ((w (sget (k-ac "wt:" tok acct) "0")))
          (if (u128/eq w "0")
              "0" ;; no buys (or already claimed) → pays 0
              (let ((ws (sget (k-ws tok) "0")))
                (if (u128/eq ws "0")
                    "0"
                    (let ((pot (sget (k-po tok) "0")))
                      (let ((snum (u128/mul pot w)))
                        (let ((share (u128/div snum ws)))
                          (let ((pot2 (u128/sub pot share))
                                (ws2  (u128/sub ws w)))
                            (sput (k-po tok) pot2)
                            (sput (k-ac "wt:" tok acct) "0")
                            (sput (k-ws tok) ws2)
                            (if (u128/eq share "0")
                                0
                                (pay-near acct share))
                            share)))))))))))

;; ═══════════ export wrappers (wasm env reads; interp tests hit cores) ═══════════

(define (launch)
  (let ((tok (default (near/json_get_str "token") ""))
        (sen-i (near/json_get_int "seniority"))
        (gm-i (near/json_get_int "gate_ms"))
        (dep (near/attached_deposit_u128))
        (ts (near/block_timestamp)))
    (let ((sen (u128/from-i64 sen-i)))
      (let ((gms (u128/from-i64 gm-i)))
        (let ((gms2 (if (= gms "0") GATE_MS gms)))
          (let ((r (launch-core tok sen gms2 dep ts)))
            (near/return r)))))))

(define (buy)
  (let ((tok (default (near/json_get_str "token") ""))
        (mo (default (near/json_get_str "min_out") "0"))
        (dep (near/attached_deposit_u128))
        (acct (near/predecessor_account_id))
        (ts (near/block_timestamp)))
    (let ((out (buy-core tok acct dep mo ts)))
      (near/return out))))

(define (quote-buy)
  (let ((tok (default (near/json_get_str "token") ""))
        (ni (default (near/json_get_str "near_in") "0")))
    (let ((pn (sget (k-pn tok) "")))
      (if (= pn "")
          (near/return "null")
          (let ((out (calc-buy-out tok ni)))
            (let ((j1 (json-set "{}" "near_in" (str-cat "\"" ni "\""))))
              (let ((j2 (json-set j1 "tokens_out" (str-cat "\"" out "\""))))
                (near/return j2))))))))

(define (ft-on-transfer)
  ;; NEP-141: the token contract is the predecessor
  (let ((tok (near/predecessor_account_id))
        (sender (default (near/json_get_str "sender_id") ""))
        (amount (default (near/json_get_str "amount") "0"))
        (msg (default (near/json_get_str "msg") ""))
        (ts (near/block_timestamp)))
    (let ((mo (if (= msg "")
                  "0"
                  (let ((r (json-get-str "min_out" msg)))
                    (if (= r "") "0" r)))))
      (let ((mx (if (= msg "")
                    "0"
                    (let ((r2 (json-get-str "max_near" msg)))
                      (if (= r2 "") "0" r2)))))
        (let ((keep (sell-core tok sender amount msg mo mx ts)))
          (near/return keep))))))

(define (claim-seniority)
  (let ((tok (default (near/json_get_str "token") ""))
        (acct (near/predecessor_account_id)))
    (let ((sh (claim-core tok acct)))
      (near/return sh))))

(define (get-pool)
  (let ((tok (default (near/json_get_str "token") "")))
    (let ((pn (sget (k-pn tok) "")))
      (if (= pn "")
          (near/return "null")
          (let ((pt (sget (k-pt tok) "0")))
            (let ((j1 (json-set "{}" "near" (str-cat "\"" pn "\""))))
              (let ((j2 (json-set j1 "tokens" (str-cat "\"" pt "\""))))
                (near/return j2))))))))

(define (get-taxes)
  (let ((tok (default (near/json_get_str "token") ""))
        (acct (near/predecessor_account_id))
        (ts (near/block_timestamp)))
    (let ((pn (sget (k-pn tok) "")))
      (if (= pn "")
          (near/return "null")
          (let ((lbt (sget (k-ac "lb:" tok acct) ""))
                (rab (sget (k-ac "rb:" tok acct) ""))
                (pot (sget (k-po tok) "0"))
                (ws  (sget (k-ws tok) "0")))
            (let ((fb (flip-bp-of lbt ts))
                  (mb (mcap-bp-of rab pn)))
              (let ((j1 (json-set "{}" "flip_bp" fb)))
                (let ((j2 (json-set j1 "mcap_bp" mb)))
                  (let ((j3 (json-set j2 "pot" (str-cat "\"" pot "\""))))
                    (let ((j4 (json-set j3 "weights_sum" (str-cat "\"" ws "\""))))
                      (near/return j4)))))))))))

(export "launch" launch)
(export "buy" buy)
(export "quote_buy" quote-buy #t)
(export "ft_on_transfer" ft-on-transfer)
(export "claim_seniority" claim-seniority)
(export "get_pool" get-pool #t)
(export "get_taxes" get-taxes #t)
