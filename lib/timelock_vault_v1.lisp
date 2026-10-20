;; ═══════════════════════════════════════════════════════════════════════
;; lib/timelock_vault_v1.lisp — absolute-ns timelock vault (2026-10-16)
;;
;; Smallest real custody contract: deposit an ft2 token, locked until an
;; absolute NEAR timestamp; withdraw only after unlock. The first lisp-side
;; consumer of (near/block_timestamp).
;;
;; ABI (Option A ruling, 2026-08-26 — pinned by tests/test_timelock_vault.rs
;; path 8): block_timestamp returns an EXACT DECIMAL STRING (raw i64 ns
;; routed through i64→itoa; nanos ~1.8e18 exceed the 61-bit tagged num
;; payload). It stays in the string family end to end — compared with
;; li-cmp, never to-string/str->num (to-string on an already-str value
;; untags as NUM and emits heap-pointer garbage; ns > 2^60 overflows
;; str->num's checked domain).
;;
;; State (near/store-bytes / near/load-bytes):
;;   VL|<tok>|<owner> = "<amt>|<unlock_ns>"   open lock record
;;     (split at FIRST | — amounts never contain |; same packing rule as
;;      v5's acc|rm, clmm_pool_v5.lisp ~75)
;;   ESC|<tok> = escrow float mirror
;;     (C2b discipline: escrow is credited ONLY by ft_on_transfer, drained
;;      ONLY by payout successes — ESC == Σ open VL records at all times)
;;
;; Entry — ft_on_transfer ({"sender_id","amount","msg"}), msg JSON:
;;   {"op":"lk","u":"<absolute unlock ns>"} — owner = sender_id (the
;;   depositor; predecessor here is the TOKEN contract — the wd guard's
;;   predecessor must be able to match the record). Rejects: u ≤
;;   block_timestamp() (past unlock = instant vault, pointless and
;;   ambiguous), amt 0 / non-numeric, unknown op / malformed msg →
;;   near/return amt (REJECT + zero state change, identity discipline —
;;   resolve refunds the leg whole, used:0).
;;
;; Ops — exported "_run" (near/input JSON, op dispatch like v5 ~1071):
;;   ts   — Phase-0 ABI pin: returns (near/block_timestamp) verbatim.
;;   wd   {"tok"} — caller (predecessor) must match the record owner.
;;          Guard: block_timestamp() ≥ unlock_ns (boundary INCLUSIVE).
;;          Payout = single-leg ft_transfer promise to owner (the C2b
;;          claim pattern; two legs are never atomic). Record + escrow
;;          are drained ONLY in the pay success branch — wd itself is
;;          stateless, so a failed payout receipt leaves lock + escrow
;;          intact for retry. Cleared/absent record ⇒ WD_NONE reject;
;;          a second wd after payout hits the same reject (no double pay).
;;   view {"tok","who"} — record + escrow readout (test helper):
;;          "amt:<amt>:u:<unlock>:esc:<escrow>" (amt 0 ⇒ no open record).
;;
;; Non-goals v1: no beneficiary ≠ depositor, no partial withdrawal, no
;; extend, no cancel.
;;
;; Call discipline (compiler note, v5 header): one value-producing call
;; per let binding; sequential single-binding lets; `begin` wraps only
;; effect-only sequences. All custody math is li-* decimal-string ops
;; (limb_math value primitives — arbitrary precision, no 2^60 ceiling).
;; ═══════════════════════════════════════════════════════════════════════

(define (v1-bz k) (let ((v (near/load-bytes k))) (if (nil? v) "0" v)))
(define (v1-put k v) (near/store-bytes k v))
(define (v1-fail m) (near/abort m))

;; li helpers (limb_math string ops — same 3 as v5/clmm_core)
(define (li-add a b)
  (let ((A (str->limbs a)) (B (str->limbs b)) (r (buf-alloc 64)))
    (limbs->str r (limb-add (car A) (car B) r (car (cdr A)) (car (cdr B))))))
(define (li-sub a b)
  (let ((A (str->limbs a)) (B (str->limbs b)) (r (buf-alloc 64)))
    (limbs->str r (limb-sub (car A) (car B) r (car (cdr A)) (car (cdr B))))))
(define (li-cmp a b)
  (let ((A (str->limbs a)) (B (str->limbs b)))
    (limb-cmp (car A) (car B) (car (cdr A)) (car (cdr B)))))

;; "amt|unlock" packing: split at the FIRST | (amounts never contain |)
(define (v1-fst res)
  (loop ((i 0))
    (if (str= (str-slice res i (+ i 1)) "|")
        (str-slice res 0 i)
        (recur (+ i 1)))))
(define (v1-snd res)
  (let ((k (str-len res)))
    (loop ((i (- k 1)))
      (if (str= (str-slice res i (+ i 1)) "|")
          (str-slice res (+ i 1) k)
          (recur (- i 1))))))

;; <=40 digits, ASCII only — OOB-parse & u128-scale guard (v4 pattern)
(define (v1-num-ok? s)
  (if (> (str-length s) 40)
      "0"
      (loop ((i 0))
        (if (>= i (str-length s))
            "1"
            (let ((d (- (byte-at s i) 48)))
              (if (or (< d 0) (> d 9))
                  "0"
                  (recur (+ i 1))))))))

(define (v1-vlkey tok owner) (str-cat "VL|" (str-cat tok (str-cat "|" owner))))
(define (v1-esckey tok) (str-cat "ESC|" tok))

;; ── Phase-0 ABI pin: the exact number→string path the guard uses ──────
(define (v1-ts) (near/block_timestamp))

;; ── ft_on_transfer: the lock entry ─────────────────────────────────────
(define (v1-on-transfer)
  (let ((inp (near/input)))
  (let ((sender (json-get-str "sender_id" inp)))
  (let ((amt (json-get-str "amount" inp)))
  (let ((msg (json-get-str "msg" inp)))
    (if (str= (v1-num-ok? amt) "0")
        (near/return amt)                            ;; malformed amount: reject whole
    (if (not (= (li-cmp amt "0") 1))
        (near/return amt)                            ;; zero deposit: reject
    (if (not (str= (json-get-str "op" msg) "lk"))
        (near/return amt)                            ;; unknown/malformed msg: reject
    (let ((u (json-get-str "u" msg)))
    (if (str= (v1-num-ok? u) "0")
        (near/return amt)                            ;; non-numeric unlock: reject
    (if (not (= (li-cmp (v1-ts) u) -1))              ;; u ≤ ts (string-domain cmp)
        (near/return amt)                            ;; past unlock: reject
    (let ((tok (near/predecessor_account_id)))
    (let ((k (v1-vlkey tok sender)))
    (begin
      (v1-put k (str-cat amt (str-cat "|" u)))
      (v1-put (v1-esckey tok) (li-add (v1-bz (v1-esckey tok)) amt))
      (near/return "0")))))))))))))))                 ;; full custody: used = amt

;; ── wd: guard + single-leg payout promise ──────────────────────────────
;; wd itself writes NOTHING — the pay callback owns record+escrow on the
;; success branch only (failed payout ⇒ state intact, retry succeeds).
;; Record presence is checked with str= (NOT li-cmp — a packed
;; "amt|unlock" record contains '|', which limb parsing can't read).
(define (v1-wd)
  (let ((inp (near/input)))
  (let ((who (near/predecessor_account_id)))
  (let ((tok (json-get-str "tok" inp)))
  (let ((k (v1-vlkey tok who)))
  (let ((rec (v1-bz k)))
    (if (str= rec "0")
        (v1-fail "WD_NONE")                          ;; no/cleared record ⇒ reject
    (let ((amt (v1-fst rec)))
    (let ((u (v1-snd rec)))
      (if (= (li-cmp (v1-ts) u) -1)
          (v1-fail "WD_EARLY")                       ;; ts < unlock_ns ⇒ reject
      (let ((p (near/promise_batch_create tok)))
        (begin
          (near/promise_batch_action_function_call p "ft_transfer"
            (json-set (json-set "{}" "receiver_id" who) "amount" amt)
            "0" 40000000000000)
          (near/promise_return
            (near/promise_then p (near/current_account_id) "pay"
              (json-set (json-set (json-set (json-set "{}" "k" tok)
                "r" who) "a" amt) "v" k)
              "0" 5000000000000))
          "wd-sent"))))))))))))

;; ── pay callback: drain on success, restore-nothing on failure ────────
;; Self-call gate (v5 p5-pay pattern). Failure path touches NO state —
;; the record + escrow survive verbatim (retry the wd).
(define (v1-pay)
  (if (str= (near/predecessor_account_id) (near/current_account_id))
      (let ((inp (near/input)))
        (let ((res (near/promise_result 0)))
          (if (< (str-length res) 1)
              (near/return "wd-failed")              ;; payout receipt failed: state intact
              (begin
                (v1-put (json-get-str "v" inp) "0")  ;; clear record ("0" = bz-missing marker)
                (v1-put (v1-esckey (json-get-str "k" inp))
                        (li-sub (v1-bz (v1-esckey (json-get-str "k" inp)))
                                (json-get-str "a" inp)))
                (near/return "0")))))
      (near/return "forbidden")))

;; ── view: record + escrow readout (test helper) ───────────────────────
(define (v1-view)
  (let ((inp (near/input)))
  (let ((tok (json-get-str "tok" inp)))
  (let ((who (json-get-str "who" inp)))
  (let ((rec (v1-bz (v1-vlkey tok who))))
  (let ((esc (v1-bz (v1-esckey tok))))
    (if (str= rec "0")
        (str-cat "amt:0:u:0:esc:" esc)
        (str-cat "amt:" (str-cat (v1-fst rec)
                   (str-cat ":u:" (str-cat (v1-snd rec)
                     (str-cat ":esc:" esc)))))))))))

;; ── dispatch ───────────────────────────────────────────────────────────
(define (v1-run)
  (let ((op (json-get-str "op" (near/input))))
    (if (str= op "ts") (v1-ts)
    (if (str= op "wd") (v1-wd)
    (if (str= op "view") (v1-view)
        "unknown-op")))))

(export "_run" v1-run)
(export "ft_on_transfer" v1-on-transfer)
(export "pay" v1-pay)
