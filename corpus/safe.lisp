;; ═══════════════════════════════════════════════════════════════════
;; corpus/safe.lisp — corpus contract #2: 2-of-3 multisig vault
;;
;; STATE MODEL — all state in storage:
;;   "own:<account>"            → 1      (owner set, written by init)
;;   "tx:<id>:to:<recipient>"   → 1      (sealed intent: this key exists
;;                                        iff recipient was proposed for id)
;;   "tx:<id>:lo" / "tx:<id>:hi"→ Num    (amount split: 18-digit low word
;;                                        + high word, both < 2^60)
;;   "ap:<id>:<account>"        → 1      (approval slot per owner)
;;
;; MATH — amounts are u128 decimal STRINGS end-to-end (yoctoNEAR 10^24
;; scale). NO bare + on amounts. Words stored as Nums via u128/to-i64
;; (checked ≤ 2^60-1 by construction: lo < 10^18 < 2^60, hi ≤ total cap).
;; Recombine ONLY via u128/{mul,add,from-i64}.
;;
;; STRING EQUALITY — none anywhere (wasm path has no string content-eq;
;; erc20 corpus rule). Owner/recipient/tx-id identity is storage-key
;; existence (tamper-proof KV): (near/has_key k) / (near/load k).
;;
;; ERROR MODEL (erc20 corpus convention) — mutating ops return "" on
;; failure, non-empty string on success. All checks precede all stores:
;; a refused op leaves state UNCHANGED. (fail? v) = (= (str-length v) 0).
;;
;; v1 scope: fixed owner trio set at init; propose carries the proposer's
;; approval implicitly; execute needs ≥ 2 distinct slots; cancel is 3/3.

;; ── amount words (61-bit discipline) ───────────────────────────
(define AMT_BASE "1000000000000000000") ; 10^18
(define AMT_MAX  "1000000000000000000000000000000000000") ; 10^36 hard cap

;; (amount-ok? a) — explicit comparisons, no truthiness
(define (amount-ok? a)
  (and (u128/gt a "0") (u128/lt a AMT_MAX)))

(define (store-amount! id a)
  (begin
    (near/store (str-cat "tx:" (str-cat id ":lo")) (u128/to-i64 (u128/mod a AMT_BASE)))
    (near/store (str-cat "tx:" (str-cat id ":hi")) (u128/to-i64 (u128/div a AMT_BASE)))
    "1"))

;; recombine — words are Nums; all summation in u128-string space
(define (num->u128 n) (u128/from-i64 n))
(define (load-amount id)
  (u128/add
    (u128/mul (num->u128 (near/load (str-cat "tx:" (str-cat id ":hi")))) AMT_BASE)
    (num->u128 (near/load (str-cat "tx:" (str-cat id ":lo"))))))

;; ── identity via storage keys ──────────────────────────────────
(define (is-owner? acct)
  (= 1 (near/load (str-cat "own:" acct))))
(define (signer-is-owner?)
  (is-owner? (near/signer_account_id)))

;; ── init: seal the fixed owner trio (v1) ────────────────────────
(define O1 "alice.near")
(define O2 "bob.near")
(define O3 "carol.near")

(define (init)
  (if (= 1 (near/load "own:init-done"))
      "" ; idempotence: never re-init
      (begin
        (near/store "own:init-done" 1)
        (near/store (str-cat "own:" O1) 1)
        (near/store (str-cat "own:" O2) 1)
        (near/store (str-cat "own:" O3) 1)
        "1")))

;; ── propose ────────────────────────────────────────────────────
;; (propose "id" "recipient" "amount") — any owner; recipient sealed
;; into the key; proposer's approval auto-recorded.
(define (propose id recipient amount)
  (if (not (signer-is-owner?)) ""
  (if (not (amount-ok? amount)) ""
  (if (near/has_key (str-cat "ap:" (str-cat id (str-cat ":" (near/signer_account_id)))))
      "" ; id already used by this owner
      (begin
        (near/store (str-cat "tx:" (str-cat id (str-cat ":to:" recipient))) 1)
        (store-amount! id amount)
        (near/store (str-cat "ap:" (str-cat id (str-cat ":" (near/signer_account_id)))) 1)
        amount)))))

;; ── approve ────────────────────────────────────────────────────
;; idempotent per owner: re-approval is a no-op success (slot already 1)
(define (approve id recipient)
  (if (not (signer-is-owner?)) ""
  (if (not (near/has_key (str-cat "tx:" (str-cat id (str-cat ":to:" recipient)))))
      "" ; tx unknown OR recipient mismatch — same refusal
      (begin
        (near/store (str-cat "ap:" (str-cat id (str-cat ":" (near/signer_account_id)))) 1)
        "1"))))

(define (approval-count id)
  (+ (near/load (str-cat "ap:" (str-cat id (str-cat ":" O1))))
     (+ (near/load (str-cat "ap:" (str-cat id (str-cat ":" O2))))
        (near/load (str-cat "ap:" (str-cat id (str-cat ":" O3)))))))

;; ── execute (≥2 slots) ─────────────────────────────────────────
(define (cleanup! id recipient)
  (begin
    (near/remove (str-cat "tx:" (str-cat id ":lo")))
    (near/remove (str-cat "tx:" (str-cat id ":hi")))
    (near/remove (str-cat "ap:" (str-cat id (str-cat ":" O1))))
    (near/remove (str-cat "ap:" (str-cat id (str-cat ":" O2))))
    (near/remove (str-cat "ap:" (str-cat id (str-cat ":" O3))))
    (near/remove (str-cat "tx:" (str-cat id (str-cat ":to:" recipient))))
    "1"))

(define (execute id recipient)
  (if (not (signer-is-owner?)) ""
  (if (not (near/has_key (str-cat "tx:" (str-cat id (str-cat ":to:" recipient)))))
      ""
  (if (< (approval-count id) 2)
      ""
      (begin
        (near/transfer_u128 recipient (load-amount id))
        (cleanup! id recipient))))))

;; ── cancel (unanimous 3/3) ─────────────────────────────────────
(define (cancel id recipient)
  (if (not (signer-is-owner?)) ""
  (if (not (near/has_key (str-cat "tx:" (str-cat id (str-cat ":to:" recipient)))))
      ""
  (if (< (approval-count id) 3)
      ""
      (cleanup! id recipient)))))

;; ── views ──────────────────────────────────────────────────────
(define (tx-amount id) (load-amount id))   ; decimal string
(define (approvals id) (approval-count id)) ; 0..3
