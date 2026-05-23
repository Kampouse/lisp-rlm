;;
;; Orderbook v4.2 — Fix auto-tagging of numeric literals
;;
;; In lisp-rl, numeric literals are UNTAGGED values that the compiler
;; auto-tags via (val << 3) | TAG_NUM. So literal 1 becomes 8 (tagged 1),
;; literal 0 becomes 0, literal 100 becomes 800 (tagged 100).
;;
;; All intrinsics (shl, bor, =, <, >, +, -, etc.) take tagged inputs
;; and produce tagged outputs. But the COMPILER tags literal values.
;; So (shl ep 48) means: shift untagged_ep left by untagged 48 positions.
;; The compiler tags 48 → (48<<3)=384, then shl untags both args.
;;
;; For near/store_num and near/load_num: keys are TAGGED on the stack,
;; un-tagged internally before storage. Values are TAGGED on the stack,
;; stored as-is in NEAR storage.
;;
;; Key layout (61-bit safe):
;;   [epoch:8 | type:8 | price:14 | pad:2 | field:8]
;;   Bits:  48-55    40-47     12-25   10-11    0-7
;;
;; Special key 0 (untagged) = current_epoch
;; Field values: 1=best_bid_price, 2=best_ask_price (global)
;;               1=depth, 2=next_price (ask/bid levels)
;;
;; NEAR view/change wrappers tag input args with <<3
;; and un-tag return values with >>3.
;; RPC calls pass RAW (untagged) i64 values.

;; === Epoch ===
;; near/load_num returns 0 (TAG_NIL) when key not found
;; (= ep 0): ep is tagged, 0 tags to 0 → correct comparison
;; Initial epoch should be 1 (compiler tags 1 → 8 = tagged epoch 1)
(define (get_epoch)
  (let ((ep (near/load_num 0)))
    (if (= ep 0) 1 ep)))

(define (ensure_epoch)
  (let ((ep (near/load_num 0)))
    (if (= ep 0)
      (begin (near/store_num 0 1) 1)
      ep)))

;; === Key constructors (tagged inputs, tagged output) ===
;; shl and bor auto-untag, compute, auto-tag their results.
;; Literal 48 → tagged 384 → untagged 48 in shl. Correct.
(define (glb_key ep field)
  (bor (shl ep 48) (bor (shl 0 40) (bor (shl 0 12) field))))

(define (ask_key ep price field)
  (bor (shl ep 48) (bor (shl 1 40) (bor (shl price 12) field))))

(define (bid_key ep price field)
  (bor (shl ep 48) (bor (shl 2 40) (bor (shl price 12) field))))

;; === Global accessors ===
(define (best_bid) (near/load_num (glb_key (get_epoch) 1)))
(define (set_best_bid price) (near/store_num (glb_key (get_epoch) 1) price))
(define (best_ask) (near/load_num (glb_key (get_epoch) 2)))
(define (set_best_ask price) (near/store_num (glb_key (get_epoch) 2) price))

;; === Ask level accessors ===
(define (ask_depth price) (near/load_num (ask_key (get_epoch) price 1)))
(define (ask_next price) (near/load_num (ask_key (get_epoch) price 2)))
(define (set_ask_depth price d) (near/store_num (ask_key (get_epoch) price 1) d))
(define (set_ask_next price nxt) (near/store_num (ask_key (get_epoch) price 2) nxt))

;; === Bid level accessors ===
(define (bid_depth price) (near/load_num (bid_key (get_epoch) price 1)))
(define (bid_next price) (near/load_num (bid_key (get_epoch) price 2)))
(define (set_bid_depth price d) (near/store_num (bid_key (get_epoch) price 1) d))
(define (set_bid_next price nxt) (near/store_num (bid_key (get_epoch) price 2) nxt))

;; === Linked list: ask insertion (ascending, lowest price first) ===
(define (insert_ask_after cur price)
  (loop ((c cur))
    (let ((nxt (ask_next c)))
      (if (or (= nxt 0) (< price nxt))
        (begin (set_ask_next c price) (set_ask_next price nxt))
        (recur nxt)))))

(define (insert_ask price)
  (let ((ba (best_ask)))
    (if (= ba 0)
      (begin (set_ask_next price 0) (set_best_ask price))
      (if (< price ba)
        (begin (set_ask_next price ba) (set_best_ask price))
        (insert_ask_after ba price)))))

;; === Linked list: bid insertion (descending, highest price first) ===
(define (insert_bid_after cur price)
  (loop ((c cur))
    (let ((nxt (bid_next c)))
      (if (or (= nxt 0) (> price nxt))
        (begin (set_bid_next c price) (set_bid_next price nxt))
        (recur nxt)))))

(define (insert_bid price)
  (let ((bb (best_bid)))
    (if (= bb 0)
      (begin (set_bid_next price 0) (set_best_bid price))
      (if (> price bb)
        (begin (set_bid_next price bb) (set_best_bid price))
        (insert_bid_after bb price)))))

;; === Linked list removal ===
(define (remove_ask price)
  (let ((ba (best_ask)))
    (if (= ba price)
      (set_best_ask (ask_next price))
      (let ((prev (loop ((c ba) (p 0))
                    (let ((nxt (ask_next c)))
                      (if (= nxt price) p
                        (recur nxt c))))))
        (if (= prev 0) 0 (set_ask_next prev (ask_next price)))))))

(define (remove_bid price)
  (let ((bb (best_bid)))
    (if (= bb price)
      (set_best_bid (bid_next price))
      (let ((prev (loop ((c bb) (p 0))
                    (let ((nxt (bid_next c)))
                      (if (= nxt price) p
                        (recur nxt c))))))
        (if (= prev 0) 0 (set_bid_next prev (bid_next price)))))))

;; === Spread ===
(define (get_spread)
  (let ((ba (best_ask)) (bb (best_bid)))
    (if (or (= ba 0) (= bb 0)) 0 (- ba bb))))

;; === Clear (bump epoch by 1) ===
;; (+ ep 1): ep is tagged, 1 is tagged-8, + untags both → untagged_ep + 1, tags result
(define (clear)
  (let ((ep (get_epoch)))
    (near/store_num 0 (+ ep 1))
    0))

;; === Trading ===
(define (limit_sell oid price amt fee_bps)
  (let ((ep (ensure_epoch)))
    (loop ((a amt) (filled 0))
      (let ((bb (best_bid)))
        (if (or (= bb 0) (> price bb))
          (begin (set_ask_depth price (+ (ask_depth price) a)) (insert_ask price) (+ filled a))
          (let ((d (bid_depth bb)))
            (if (>= d a)
              (begin (if (= d a) (remove_bid bb) (set_bid_depth bb (- d a))) (+ filled a))
              (begin (set_bid_depth bb 0) (remove_bid bb) (recur (- a d) (+ filled d))))))))))

(define (limit_buy oid price amt fee_bps)
  (let ((ep (ensure_epoch)))
    (loop ((a amt) (filled 0))
      (let ((ba (best_ask)))
        (if (or (= ba 0) (< price ba))
          (begin (set_bid_depth price (+ (bid_depth price) a)) (insert_bid price) (+ filled a))
          (let ((d (ask_depth ba)))
            (if (>= d a)
              (begin (if (= d a) (remove_ask ba) (set_ask_depth ba (- d a))) (+ filled a))
              (begin (set_ask_depth ba 0) (remove_ask ba) (recur (- a d) (+ filled d))))))))))

;; === Exports ===
(export "best_bid" best_bid #t)
(export "best_ask" best_ask #t)
(export "get_spread" get_spread #t)
(export "get_epoch" get_epoch #t)
(export "limit_buy" limit_buy #f)
(export "limit_sell" limit_sell #f)
(export "clear" clear #f)
(export "bid_depth" bid_depth #t)
(export "ask_depth" ask_depth #t)
(export "bid_next" bid_next #t)
(export "ask_next" ask_next #t)