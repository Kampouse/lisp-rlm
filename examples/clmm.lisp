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
(define (initialize sp tick)
  (begin
    (near/store_num 0 sp)
    (near/store_num 1 tick)
    (near/store_num 2 0)
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
  (let ((sp (get_price))
        (liq (get_liq)))
    (if (= liq 0)
      0
      (let ((denom (+ liq (muldiv dx sp (q32)))))
        (let ((new_sp (muldiv sp liq denom)))
          (let ((dy (muldiv liq (- sp new_sp) (q32))))
            (begin
              (near/store_num 0 new_sp)
              dy)))))))


;; === Swap token1 → token0 (price increases) ===
;; (swap1 dy_in) — sell dy_in token1, receive token0.
;; Returns amount of token0 received.
(define (swap1 dy_in)
  (let ((sp (get_price))
        (liq (get_liq)))
    (if (= liq 0)
      0
      (let ((dp (muldiv dy_in (q32) liq)))
        (let ((new_sp (+ sp dp)))
          (let ((dx (muldiv liq (muldiv dp (q32) sp) new_sp)))
            (begin
              (near/store_num 0 new_sp)
              dx)))))))


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

(define (w-initialize)
  (initialize (str->num (json-get-str "sp" (near/input)))
              (str->num (json-get-str "tick" (near/input)))))
(define (w-add-liquidity)
  (add_liquidity (str->num (json-get-str "lower" (near/input)))
                 (str->num (json-get-str "upper" (near/input)))
                 (str->num (json-get-str "liq" (near/input)))))
(define (w-remove-liquidity)
  (remove_liquidity (str->num (json-get-str "lower" (near/input)))
                    (str->num (json-get-str "upper" (near/input)))
                    (str->num (json-get-str "liq" (near/input)))))
(define (w-swap0)
  (swap0 (str->num (json-get-str "dx" (near/input)))))
(define (w-swap1)
  (swap1 (str->num (json-get-str "dy_in" (near/input)))))
(define (w-tick-net)
  (tick_net (str->num (json-get-str "tick" (near/input)))))
(define (w-tick-gross)
  (tick_gross (str->num (json-get-str "tick" (near/input)))))
(define (w-sp-at-tick)
  (sp_at_tick (str->num (json-get-str "tick" (near/input)))))

;; === Exports ===
;; #t = view (read-only), #f = call (mutable)

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
