;;;
;;; CLMM v2 — Concentrated Liquidity Market Maker (U128)
;;;
;;; Clean API: borsh u128 values (16 bytes each).
;;; Internal: u128 stored as [lo, hi] arrays.
;;;
;;; Storage layout (i64 keys):
;;;   key 0: sqrt_price_lo, key 1: sqrt_price_hi
;;;   key 2: current_tick
;;;   key 3: liq_lo, key 4: liq_hi
;;;   key 5: fee0_lo, key 6: fee0_hi
;;;   key 7: fee1_lo, key 8: fee1_hi
;;;
;;; Tick state:
;;;   tick_key(tick, 0) = liquidity_net (i64)
;;;   tick_key(tick, 1) = liquidity_gross_lo
;;;   tick_key(tick, 2) = liquidity_gross_hi
;;;

(borsh-schema
  (InitArgs (sqrt_price u128) (tick i64))
  (AddLiqArgs (lower i64) (upper i64) (amount u128))
  (PoolState (sqrt_price u128) (tick i64) (liquidity u128))
)

;; === Constants ===
(define TICK_OFF 50000)

;; === Tick helpers ===
(define (tick_key tick field)
  (bor (shl (+ tick TICK_OFF) 4) field))

(define (tick_net tick)
  (near/load_num (tick_key tick 0)))

(define (set_tick_net tick val)
  (near/store_num (tick_key tick 0) val))

(define (tick_gross_lo tick)
  (near/load_num (tick_key tick 1)))

(define (tick_gross_hi tick)
  (near/load_num (tick_key tick 2)))

(define (set_tick_gross tick lo hi)
  (begin
    (near/store_num (tick_key tick 1) lo)
    (near/store_num (tick_key tick 2) hi)))

;; === Pool State ===

(define (get_price_lo) (near/load_num 0))
(define (get_price_hi) (near/load_num 1))
(define (set_price lo hi)
  (begin (near/store_num 0 lo) (near/store_num 1 hi)))

(define (get_tick) (near/load_num 2))
(define (set_tick val) (near/store_num 2 val))

(define (get_liq_lo) (near/load_num 3))
(define (get_liq_hi) (near/load_num 4))
(define (set_liq lo hi)
  (begin (near/store_num 3 lo) (near/store_num 4 hi)))

;; === Pool Initialization ===
;; Borsh: sqrt_price (16 bytes) + tick (8 bytes) = 24 bytes
;; sqrt_price comes as array [lo, hi] from borsh-deserialize

(define (initialize)
  (let ((input (near/input)))
    ;; input is tagged ptr to TEMP_MEM with borsh bytes
    ;; borsh-deserialize u128 returns array [count, lo, hi]
    (let ((sqrt_price (borsh-deserialize "InitArgs" input 0)))
      ;; sqrt_price is array [count, lo, hi]
      (set_price
        (vec-nth sqrt_price 1)
        (vec-nth sqrt_price 2))
      ;; tick is at offset 16 in borsh (after u128)
      (let ((tick (borsh-deserialize "InitArgs" input 16)))
        (set_tick tick)
        (set_liq 0 0)
        0))))

;; === Add Liquidity ===
;; Borsh: lower (8) + upper (8) + amount (16) = 32 bytes

(define (add_liquidity)
  (let ((input (near/input)))
    (let ((lower (borsh-deserialize "AddLiqArgs" input 0))
          (upper (borsh-deserialize "AddLiqArgs" input 8))
          (amount (borsh-deserialize "AddLiqArgs" input 16)))
      ;; amount is array [count, lo, hi]
      (let ((amt_lo (vec-nth amount 1))
            (amt_hi (vec-nth amount 2)))
        ;; Update tick_net (i64)
        (set_tick_net lower (- (tick_net lower) amt_lo))
        (set_tick_net upper (+ (tick_net upper) amt_lo))
        ;; Update tick_gross (u128 as lo/hi)
        (set_tick_gross lower
          (+ (tick_gross_lo lower) amt_lo)
          (+ (tick_gross_hi lower) amt_hi))
        (set_tick_gross upper
          (+ (tick_gross_lo upper) amt_lo)
          (+ (tick_gross_hi upper) amt_hi))
        ;; If current tick in range, add to active liquidity
        (if (>= (get_tick) lower)
          (if (< (get_tick) upper)
            (set_liq
              (+ (get_liq_lo) amt_lo)
              (+ (get_liq_hi) amt_hi))
            0)
          0)
        ;; Return amount_lo
        amt_lo))))

;; === Remove Liquidity ===

(define (remove_liquidity)
  (let ((input (near/input)))
    (let ((lower (borsh-deserialize "AddLiqArgs" input 0))
          (upper (borsh-deserialize "AddLiqArgs" input 8))
          (amount (borsh-deserialize "AddLiqArgs" input 16)))
      (let ((amt_lo (vec-nth amount 1))
            (amt_hi (vec-nth amount 2)))
        (set_tick_net lower (+ (tick_net lower) amt_lo))
        (set_tick_net upper (- (tick_net upper) amt_lo))
        (set_tick_gross lower
          (- (tick_gross_lo lower) amt_lo)
          (- (tick_gross_hi lower) amt_hi))
        (set_tick_gross upper
          (- (tick_gross_lo upper) amt_lo)
          (- (tick_gross_hi upper) amt_hi))
        (if (>= (get_tick) lower)
          (if (< (get_tick) upper)
            (set_liq
              (- (get_liq_lo) amt_lo)
              (- (get_liq_hi) amt_hi))
            0)
          0)
        amt_lo))))

;; === View Functions ===

(define (get_state)
  ;; Return PoolState as borsh
  ;; sqrt_price is array [lo, hi]
  (let ((sqrt_price (list 2 (get_price_lo) (get_price_hi))))
    (borsh-serialize "PoolState" sqrt_price (get_tick) (list 2 (get_liq_lo) (get_liq_hi)))))

(export "initialize" initialize #f)
(export "add_liquidity" add_liquidity #f)
(export "remove_liquidity" remove_liquidity #f)
(export "get_state" get_state #t)