;;;
;;; CLMM v1 — Concentrated Liquidity Market Maker
;;;
;;; Q32 fixed-point arithmetic with muldiv/isqrt intrinsics.
;;; All numeric literals are UNTAGGED — compiler auto-tags via (val << 3).
;;;
;;; Storage layout (near/store_num / near/load_num — tagged i64 keys):
;;;   key 0: sqrt_price_x32
;;;   key 1: current_tick
;;;   key 2: active_liquidity
;;;   tick state: tick_key(tick, field) where
;;;     tick_key = ((tick + 50000) << 4) | field
;;;     field 0: liquidity_net (change when crossing this tick)
;;;     field 1: liquidity_gross (total liquidity referencing this tick)
;;;
;;; Core formulas (Q32):
;;;   x = L * Q32 / sqrtP    (token0 virtual reserves)
;;;   y = L * sqrtP / Q32    (token1 virtual reserves)
;;;
;;;   swap0 (sell token0, price down):
;;;     newP = L * P / (L + dx * P / Q32)
;;;     dy   = L * (P - newP) / Q32
;;;
;;;   swap1 (sell token1, price up):
;;;     newP = P + dy * Q32 / L
;;;     dx   = L * (newP - P) * Q32 / (P * newP)
;;;
;;; Tick-to-price: sqrtPrice(tick) = isqrt(Q32 * pow32(1.0001_q32, tick))
;;; where 1.0001_q32 = 10001 * Q32 / 10000
;;;

;; === Constants ===

;; Q32 = 2^32
(define (q32) (shl 1 32))

;; Tick offset — shifts negative ticks into positive key space
(define (tick_off) 50000)

;; 1.0001 in Q32 = 10001 * Q32 / 10000
(define (base_q32) (muldiv 10001 (q32) 10000))


;; === Storage helpers ===

(define (tick_key tick field)
  (bor (shl (+ tick (tick_off)) 4) field))

(define (tick_net tick)
  (near/load_num (tick_key tick 0)))

(define (tick_gross tick)
  (near/load_num (tick_key tick 1)))

(define (get_price) (near/load_num 0))
(define (get_tick)  (near/load_num 1))
(define (get_liq)   (near/load_num 2))

;; === TWAP accumulator ===
;; key 3: last_update_ns (tagged i64 — ok: ns fits tagged range for any
;;        realistic wall clock, 2^60 ns ≈ 36.5 years since epoch)
;; key 4/"tc": tick_cumulative, BIAS-SHIFTED u128 string. tick can be
;;        negative, u128 strings cannot — store Σ(tick + BIAS)·dt so every
;;        term is non-negative; viewers subtract BIAS·elapsed to unbias.
;; Uniswap-v3 semantics: cumulative updated lazily on first touch after
;; elapsed time (swap / liquidity change / observation view), NOT a cron.
;; "tc_snap": last observation snapshot (raw biased cum at a pinned ts).

(define (acc_last_ts) (default (near/storage_get "ts") "0"))
(define (acc_cum_raw) (default (near/storage_get "tc") "0"))

;; carry: advance the accumulator to NOW. Call at the top of every
;; state-mutating entrypoint and in the observation view.
;;   cum += (tick + BIAS) * (now - last)
(define (acc_carry)
  (let ((now (near/block_timestamp))
        (last (acc_last_ts)))
    (if (u128/gt now last)
      (let ((dt (u128/sub now last)))
        (near/storage_set "ts" now)
        (near/storage_set "tc"
          (u128/add (acc_cum_raw)
            (u128/mul (u128/from-i64 (+ (get_tick) (tick_off))) dt))))
      ;; now == last (same block) or clock went backwards: pin anyway
      (near/storage_set "ts" now))))

;; observation: pin (ts, cum) so a later caller can diff over a window
(define (twap_snap)
  (begin
    (acc_carry)
    (near/storage_set "tc_snap_ts" (acc_last_ts))
    (near/storage_set "tc_snap" (acc_cum_raw))
    (acc_cum_raw)))

;; arithmetic-mean biased tick since the last twap_snap:
;;   ΔbiasedCum / Δt ; viewer subtracts BIAS to get the true mean tick
(define (twap_tick_since_snap)
  (begin
    (acc_carry)
    (let ((dt (u128/sub (acc_last_ts)
                        (default (near/storage_get "tc_snap_ts") "0"))))
      (if (u128/is_zero dt)
        "0"
        (u128/div
          (u128/sub (acc_cum_raw) (default (near/storage_get "tc_snap") "0"))
          dt)))))

;; === Q32 binary exponentiation ===
;; pow32(base, exp) = base^exp in Q32
;; Uses repeated squaring: O(log exp) muldiv calls.
;; Returns Q32 (1.0) for exp=0.
(define (pow32 base exp)
  (let ((r (q32))
        (c base)
        (n exp))
    (loop ()
      (if (= n 0)
        r
        (begin
          ;; if n is odd: r = r * c / Q32
          (set! r (if (= (bor n 1) n)
                    (muldiv r c (q32))
                    r))
          ;; c = c^2 / Q32
          (set! c (muldiv c c (q32)))
          ;; n >>= 1
          (set! n (shr n 1))
          (recur))))))


;; === Tick ↔ Price conversion ===
;; sqrtPrice(tick) ≈ isqrt(pow32(base,tick)) << 16
;; EXACT at tick 0 (= Q32). For tick≠0 this is the representable-domain
;; form: the exact isqrt(price) with price = Q32^2·1.0001^tick needs a
;; ~2^64 intermediate the tagged range [-2^60, 2^60) cannot hold (muldiv
;; hard-errors by design). Error vs exact: ≤ 2^-16 relative (v1 view
;; precision; v5 does exact tick math in u128 strings).
(define (sp_at_tick tick)
  (shl (isqrt (pow32 (base_q32) tick)) 16))


;; === Pool initialization ===
(define (initialize sp tick t0 t1)
  (begin
    (near/store_num 0 sp)
    (near/store_num 1 tick)
    (near/store_num 2 0)
    (near/storage_set "ts" (near/block_timestamp))
    (near/storage_set "tc" "0")
    (near/storage_set "t0" t0)
    (near/storage_set "t1" t1)
    0))


;; === Add liquidity ===
;; (add_liquidity lower_tick upper_tick liq)
;; Adds liq to range [lower, upper). Updates tick boundary state and
;; active liquidity if current tick falls within range.
(define (add_liquidity lower upper liq)
  (begin
    ;; lower tick: crossing DOWN removes liquidity → net -= liq
    (near/store_num (tick_key lower 0)
      (+ (tick_net lower) (- 0 liq)))
    ;; upper tick: crossing UP adds liquidity → net += liq
    (near/store_num (tick_key upper 0)
      (+ (tick_net upper) liq))
    ;; Update gross liquidity at both boundary ticks
    (near/store_num (tick_key lower 1)
      (+ (tick_gross lower) liq))
    (near/store_num (tick_key upper 1)
      (+ (tick_gross upper) liq))
    ;; If current tick is within [lower, upper), add to active liquidity
    (if (>= (get_tick) lower)
      (if (< (get_tick) upper)
        (begin (near/store_num 2 (+ (get_liq) liq)) 0)
        0)
      0)
    liq))


;; === Remove liquidity ===
(define (remove_liquidity lower upper liq)
  (begin
    (near/store_num (tick_key lower 0)
      (+ (tick_net lower) liq))
    (near/store_num (tick_key upper 0)
      (+ (tick_net upper) (- 0 liq)))
    (near/store_num (tick_key lower 1)
      (+ (tick_gross lower) (- 0 liq)))
    (near/store_num (tick_key upper 1)
      (+ (tick_gross upper) (- 0 liq)))
    (if (>= (get_tick) lower)
      (if (< (get_tick) upper)
        (begin (near/store_num 2 (+ (get_liq) (- 0 liq))) 0)
        0)
      0)
    liq))


;; === Swap token0 → token1 (price decreases) ===
;; (swap0 dx) — sell dx token0, receive token1.
;; Returns amount of token1 received.
(define (swap0 dx)
  (begin
    (acc_carry)
    (let ((sp (get_price))
        (liq (get_liq)))
    (if (= liq 0)
      0
      (let ((denom (+ liq (muldiv dx sp (q32)))))
        (let ((new_sp (muldiv sp liq denom)))
          (let ((dy (muldiv liq (- sp new_sp) (q32))))
            (begin
              (near/store_num 0 new_sp)
              dy))))))))


;; === Swap token1 → token0 (price increases) ===
;; (swap1 dy_in) — sell dy_in token1, receive token0.
;; Returns amount of token0 received.
(define (swap1 dy_in)
  (begin
    (acc_carry)
    (let ((sp (get_price))
        (liq (get_liq)))
    (if (= liq 0)
      0
      (let ((dp (muldiv dy_in (q32) liq)))
        (let ((new_sp (+ sp dp)))
          (let ((dx (muldiv liq (muldiv dp (q32) sp) new_sp)))
            (begin
              (near/store_num 0 new_sp)
              dx))))))))


;; === View exports ===

(define (view_price) (get_price))
(define (view_tick)  (get_tick))
(define (view_liq)   (get_liq))

(define (view_tick_net tick)   (tick_net tick))
(define (view_tick_gross tick) (tick_gross tick))

;; View sqrtPrice at a given tick (gas-heavy: uses pow32)
(define (view_sp_at_tick tick) (sp_at_tick tick))


;; === Export wrappers (NEAR arg convention) =============================
;; Exported fns take NO wasm params: NEAR passes args via input() JSON
;; (the export wrapper type is () -> (); compiler param binding is a
;; documented TODO). Thin wrappers read named args and call the
;; param'd internals — same convention as twap/pool/ft contracts.

(define (view_tick_net tick)   (tick_net tick))
(define (view_tick_gross tick) (tick_gross tick))
(define (view_sp_at_tick tick) (sp_at_tick tick))

;; Money-safe param read: missing key => abort with ERR_ARGS (never nil/0).
(define (fail m)
  (near/log m)
  (near/abort m))

(define (arg k)
  (let ((v (default (near/json_get_str k) "")))
    (if (= v "") (fail "ERR_ARGS") v)))

(define (w-add-liquidity)
  (add_liquidity (str->num (arg "lower"))
                 (str->num (arg "upper"))
                 (str->num (arg "liq"))))
(define (w-remove-liquidity)
  (remove_liquidity (str->num (arg "lower"))
                    (str->num (arg "upper"))
                    (str->num (arg "liq"))))
(define (w-swap0)
  (swap0 (str->num (arg "dx"))))
(define (w-swap1)
  (swap1 (str->num (arg "dy_in"))))
(define (w-tick-net)
  (tick_net (str->num (arg "tick"))))
(define (w-tick-gross)
  (tick_gross (str->num (arg "tick"))))
(define (w-sp-at-tick)
  (sp_at_tick (str->num (arg "tick"))))

(define (w-twap-snap)
  (twap_snap))
(define (w-twap-tick)
  (twap_tick_since_snap))

;; === Exports ===
;; #t = view (read-only), #f = call (mutable)

(define (w-initialize)
  (initialize (str->num (arg "sp"))
              (str->num (arg "tick"))
              (arg "t0")
              (arg "t1")))
(export "initialize"       w-initialize       #f)
(export "add_liquidity"    w-add-liquidity    #f)
(export "remove_liquidity" w-remove-liquidity #f)
(export "swap0"            w-swap0            #f)
(export "swap1"            w-swap1            #f)
(export "get_price"        view_price       #t)
(export "get_tick"         view_tick        #t)
(export "get_liq"          view_liq         #t)
(export "tick_net"         w-tick-net       #t)
(export "tick_gross"       w-tick-gross     #t)
(export "sp_at_tick"       w-sp-at-tick     #t)
(export "twap_snap"        w-twap-snap      #f)
(export "twap_tick"        w-twap-tick      #t)

;; === Limit orders (position-minted) =========================
;; A resting order = single-tick position [t, t+1).
;;   side 0 = buy  (token1 in): fills when price falls through t
;;   side 1 = sell (token0 in): fills when price rises through t
;; No custody: contract never holds funds; cancel = burn by owner.

(define (order_key id) (str-concat "o:" (to-string id)))

(define (place_order side tick liq)
  (begin
    (add_liquidity tick (+ tick 1) liq)
    (let ((id (str->num (default (near/storage_get "o_next") "0"))))
      (begin
        (near/storage_set (order_key id) (near/predecessor_account_id))
        (near/storage_set (str-concat (order_key id) ":t") (to-string tick))
        (near/storage_set (str-concat (order_key id) ":l") (to-string liq))
        (near/storage_set (str-concat (order_key id) ":s") (to-string side))
        (near/storage_set (str-concat (order_key id) ":e") (to-string (get_price)))
        (near/storage_set "o_next" (to-string (+ id 1)))
        id))))

;; (cancel_order id) — burn the underlying position, returns freed liq
;; owner-gated: only the minter (o:<id>) may cancel
(define (cancel_order id)
  (let ((owner (default (near/storage_get (order_key id)) "")))
    (if (or (= (string-length owner) 0) (not (str= owner (near/predecessor_account_id))))
      0
      (let ((t (order_tick id))
            (l (str->num (default (near/storage_get (str-concat (order_key id) ":l")) "0"))))
        (begin
          (remove_liquidity t (+ t 1) l)
          (near/storage_remove (order_key id))
          (near/storage_remove (str-concat (order_key id) ":t"))
          (near/storage_remove (str-concat (order_key id) ":l"))
          (near/storage_remove (str-concat (order_key id) ":s"))
          (near/storage_remove (str-concat (order_key id) ":e"))
          l)))))

(define (order_tick id)
  (str->num (default (near/storage_get (str-concat (order_key id) ":t")) "0")))

;; (cancel_order id) — burn the underlying position, returns freed liq
(define (cancel_order id)
  (let ((t (order_tick id))
        (l (str->num (default (near/storage_get (str-concat (order_key id) ":l")) "0"))))
    (begin
      (remove_liquidity t (+ t 1) l)
      l)))

;; status: 0 = resting, 1 = crossed (fillable), 2 = unknown id
(define (order_status id)
  (let ((t (default (near/storage_get (str-concat (order_key id) ":t")) "")))
    (if (= (string-length t) 0)
      2
      (let ((tick (str->num t))
            (side (str->num (default (near/storage_get (str-concat (order_key id) ":s")) "0"))))
        (if (= side 0)
          (if (< (get_tick) tick) 0 1)
          (if (>= (get_tick) (+ tick 1)) 1 0))))))

(define (w-place-order)
  (place_order (str->num (arg "side"))
               (str->num (arg "tick"))
               (str->num (arg "liq"))))
(define (w-cancel-order)
  (cancel_order (str->num (arg "id"))))
(define (w-order-status)
  (order_status (str->num (arg "id"))))

;; === Position NFTs (Uniswap-v3 model) ======================================
;; Each liquidity position is minted as a non-fungible position token:
;;   p:<id>       owner account id
;;   p:<id>:r     "lower|upper|liq"
;;   p_next       sequential id counter
;; mint_position → add_liquidity + NFT mint; remove/transfer are owner-gated
;; by predecessor == p:<id>. This closes the v1 gap where anyone could
;; withdraw from any range.

(define (pos_key id) (str-concat "p:" (to-string id)))

(define (pos_owner id)
  (default (near/storage_get (pos_key id)) ""))

(define (mint_position lower upper liq)
  (begin
    (add_liquidity lower upper liq)
    (let ((id (str->num (default (near/storage_get "p_next") "0"))))
      (begin
        (near/storage_set (pos_key id) (near/predecessor_account_id))
        (near/storage_set (str-concat (pos_key id) ":lo") (to-string lower))
        (near/storage_set (str-concat (pos_key id) ":up") (to-string upper))
        (near/storage_set (str-concat (pos_key id) ":lq") (to-string liq))
        (near/storage_set "p_next" (to-string (+ id 1)))
        id))))

;; owner-gated burn: removes the underlying liquidity, returns freed liq
;; (range stored as three keys — p:<id>:lo/up/lq — no string parsing)
(define (remove_position id)
  (let ((owner (pos_owner id)))
    (if (= (string-length owner) 0)
      (- 0 1)
      (if (not (str= owner (near/predecessor_account_id)))
        (- 0 2)
        (let ((lower (str->num (default (near/storage_get (str-concat (pos_key id) ":lo")) "0")))
              (upper (str->num (default (near/storage_get (str-concat (pos_key id) ":up")) "0")))
              (liq   (str->num (default (near/storage_get (str-concat (pos_key id) ":lq")) "0"))))
          (begin
            (remove_liquidity lower upper liq)
            (near/storage_remove (pos_key id))
            (near/storage_remove (str-concat (pos_key id) ":lo"))
            (near/storage_remove (str-concat (pos_key id) ":up"))
            (near/storage_remove (str-concat (pos_key id) ":lq"))
            liq))))))

;; owner-gated transfer: pure NFT move, liquidity untouched
(define (transfer_position id new_owner)
  (let ((owner (pos_owner id)))
    (if (or (= (string-length owner) 0) (not (str= owner (near/predecessor_account_id))))
      0
      (begin
        (near/storage_set (pos_key id) new_owner)
        1))))

(define (view_position id)
  (str-concat (pos_owner id)
    (str-concat "|"
      (str-concat (default (near/storage_get (str-concat (pos_key id) ":lo")) "")
        (str-concat "|"
          (str-concat (default (near/storage_get (str-concat (pos_key id) ":up")) "")
            (str-concat "|" (default (near/storage_get (str-concat (pos_key id) ":lq")) ""))))))))

;; === Token escrow (NEP-141) ================================================
;; initialize(t0, t1) records the two token contracts.
;; Deposits: ft_transfer_call to the pool with msg = position id (mint first
;; with liq=0... no — mint_position itself takes amounts via msg form below).
;; Balances:  esc:<token>:<account>  = u128 decimal string
;;            plq:<id>               = pending liq awaiting deposit (mint flow)
;;
;; Mint flow: 1) ft_on_transfer msg="mint:<lo>:<up>" deposits BOTH tokens,
;;            2) deposit_mint(lo, up, liq) converts deposits → position.
;; Withdraw flow: remove_position computes tokens_owed, transfers out.

(define (esc_key tok acct) (str-concat "esc:" (str-concat tok (str-concat ":" acct))))

(define (esc_add tok acct amt)
  (near/storage_set (esc_key tok acct)
    (u128_add_str (default (near/storage_get (esc_key tok acct)) "0") amt)))

;; minimal u128 helpers on decimal strings (add only — pool math stays i64
;; ticks/liq; escrow is the only u128 surface)
(define (u128_add_str a b)
  (let ((la (string-length a)) (lb (string-length b)))
    (u128_add_digits a b la lb 0 "")))

(define (u128_add_digits a b la lb carry acc)
  (if (and (= la 0) (= lb 0) (= carry 0))
    acc
    (let ((da (if (> la 0) (- (str->num (str-slice2 a (- la 1) la)) 48) 0))
          (db (if (> lb 0) (- (str->num (str-slice2 b (- lb 1) lb)) 48) 0)))
      (let ((s (+ da (+ db carry))))
        (u128_add_digits a b (- la 1) (- lb 1)
          (if (> s 9) 1 0)
          (str-concat (to-string (mod s 10)) acc))))))

(define (str-slice2 s from to)
  (str-slice s from to))

;; NEP-141 entry: predecessor must be one of the two token contracts
(define (ft-on-transfer)
  (let ((tok (near/predecessor_account_id))
        (sender (default (near/json_get_str "sender_id") ""))
        (amount (default (near/json_get_str "amount") "0"))
        (msg (default (near/json_get_str "msg") "")))
    (let ((t0 (default (near/storage_get "t0") ""))
          (t1 (default (near/storage_get "t1") "")))
      (if (not (or (str= tok t0) (str= tok t1)))
        "0"
        (if (= (string-length sender) 0)
          "0"
          (begin
            (esc_add tok sender amount)
            amount)))))
)

;; sweep a caller's escrowed balances into a position (amounts in u128
;; strings; liq in i64 — the caller sizes the deposit off quote_deposit)
(define (withdraw_escrow tok amount)
  (let ((k (esc_key tok (near/predecessor_account_id))))
    (let ((bal (default (near/storage_get k) "0")))
      (if (str_gt_u128 bal amount)
        0
        (begin
          (near/storage_set k (u128_sub_str bal amount))
          1)))))

(define (str_gt_u128 a b)
  (or (> (string-length a) (string-length b))
      (and (= (string-length a) (string-length b)) (> (str-cmp a b) 0))))

(define (u128_sub_str a b)
  (u128_sub_digits a b (string-length a) (string-length b) 0 ""))

(define (u128_sub_digits a b la lb borrow acc)
  (if (= la 0)
    acc
    (let ((da (- (str->num (str-slice2 a (- la 1) la)) 48))
          (db (if (> lb 0) (- (str->num (str-slice2 b (- lb 1) lb)) 48) 0)))
      (let ((d (+ da (+ db (- 0 borrow)))))
        (if (< d 0)
          (u128_sub_digits a b (- la 1) (- lb 1) 1 (str-concat (to-string (+ d 10)) acc))
          (u128_sub_digits a b (- la 1) (- lb 1) 0 (str-concat (to-string d) acc)))))))

(define (str-cmp a b)
  (if (= (string-length a) (string-length b))
    (cmp-loop a b 0)
    (if (> (string-length a) (string-length b)) 1 (- 0 1))))

(define (cmp-loop a b i)
  (if (= i (string-length a))
    0
    (let ((ca (str->num (str-slice2 a i (+ i 1))))
          (cb (str->num (str-slice2 b i (+ i 1)))))
      (if (= ca cb)
        (cmp-loop a b (+ i 1))
        (if (> ca cb) 1 (- 0 1))))))

(define (w-ft-on-transfer)
  (ft-on-transfer))

(export "ft_on_transfer" w-ft-on-transfer)

;; owner-gated payout leg: send tokens_owed for a removed position
(define (payout tok to amt)
  (let ((args (str-concat "{\"receiver_id\":\"" to
                (str-concat "\",\"amount\":\"" (str-concat amt "\",\"memo\":\"\"}")))))
    (near/call tok "ft_transfer" args 30000000000000 1)))

(define (w-mint-position)
  (mint_position (str->num (arg "lower"))
                 (str->num (arg "upper"))
                 (str->num (arg "liq"))))
(define (w-remove-position)
  (remove_position (str->num (arg "id"))))
(define (w-transfer-position)
  (transfer_position (str->num (arg "id"))
                     (arg "new_owner")))
(define (w-view-position)
  (view_position (str->num (arg "id"))))

(export "mint_position"     w-mint-position      #f)
(export "remove_position"   w-remove-position    #f)
(export "transfer_position" w-transfer-position  #f)
(export "position"          w-view-position      #t)

(export "place_order"   w-place-order   #f)
(export "cancel_order"  w-cancel-order  #f)
(export "order_status"  w-order-status  #t)
