;;;
;;; nostr-gov-lisp — Phase 1
;;;
;;; Port of Kampouse/nostr-gov (Rust/near-sdk) to lisp-rlm.
;;; Nostr-keyed wallet creation with BIP-340 schnorr auth, sliding
;;; nonce window, pause/unpause, and public schnorr self-test.
;;;
;;; WIRE COMPATIBILITY (signatures valid in the Rust original are
;;; valid here — differential-tested):
;;;   owner msg  = "expires {exp}.000000000: {action} | nonce: {n} | contract: {id}"
;;;   pause msg  = "expires {exp}.000000000: pause | contract: {id}"   (no nonce!)
;;;   auth       = BIP-340 verify(pk32, sig64, SHA256(msg))
;;;
;;; Phase 1 scope (of the phased port plan):
;;;   init / create_wallet / pause / unpause / test_verify_nostr
;;;   + views get_wallet / get_owner_nonce / is_paused / get_version
;;;
;;; LANDMINE COMPLIANCE (GAPS.md + oracle rulings):
;;;   - no lambdas/closures (T4)
;;;   - explicit numeric compares ONLY (0 is truthy)
;;;   - str-cat is BINARY, nested
;;;   - block_timestamp is a decimal STRING (Option A: ns > 61-bit
;;;     tagged payload) — compared via u128/gt, never str->num
;;;   - u128 intermediates are let-bound (TEMP_MEM scratch collision)
;;;   - guarded str->num (never "" into arithmetic)
;;;   - owner bitmap is u64 → split into lo(b0..31)/hi(b32..63) pairs;
;;;     each half ≤ 2^32-1, far inside the 61-bit tagged payload.
;;;   - guard pattern: (if bad (die "..") 0) as a STATEMENT,
;;;     side effects after — never abort mixed into value branches
;;;     (near/abort is int-typed; storage_set/json_return are nil).
;;;

(define VERSION "1")
(define EMPTY "")

;; 0.5 NEAR storage deposit = 500000000000000000000000 yocto
;;   = hi<<64 | lo  → (near/deposit-gte 1001882102603448320 27105)
(define DEP_LO 1001882102603448320)
(define DEP_HI 27105)

;; allowed wallet-name chars (ASCII subset of Rust's is_alphanumeric)
(define NAME_CHARS "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_")

;; ── tiny helpers ─────────────────────────────────────────────────

;; (die msg) — log then abort. LANDMINE: near/abort's wasm emission drops
;; the message string (host_call(26) with no register plumbing), so the
;; error code is logged first for harness/differential matching. TODO
;; compiler: wire near/abort → panic_utf8 w/ message for on-chain parity.
(define (die m)
  (near/log m)
  (near/abort m))

(define (get-str k) (near/storage_get k))

(define (num-str k)
  (let ((v (get-str k)))
    (if (= (str-length v) 0) "0" v)))

(define (get-num k) (str->num (num-str k)))

;; ── sliding 64-slot nonce window (bitmap split lo/hi) ────────────
;; Rust: assert nonce >= owner_nonce, nonce < owner_nonce+64,
;; bit (nonce-owner_nonce) not already set; set it; slide while lo&1.

(define (nonce-window-check n)
  (let ((base (get-num "ononce")))
    (if (< n base)
        (die "ERR_NONCE_TOO_LOW")
        (if (>= n (+ base 64))
            (die "ERR_NONCE_WINDOW_EXCEEDED")
            0))))

;; slide while bit0 of lo is set:
;;   ononce++, lo = (lo>>1)|((hi&1)<<31), hi >>= 1
;; LANDMINE (checker): infer_let types only the FIRST body expr — a let
;; that runs statements then returns a value types as nil. Effects are
;; therefore threaded as bindings: (let ((_ (effect))) value).
(define (slide-window lo hi)
  (if (= (band lo 1) 0)
      0
      (let ((_ (near/storage_set "ononce"
                  (to-string (+ (get-num "ononce") 1)))))
        (let ((_ (near/storage_set "obm_lo"
                    (to-string (bor (shr lo 1) (shl (band hi 1) 31))))))
          (let ((_ (near/storage_set "obm_hi"
                      (to-string (shr hi 1)))))
            (slide-window (bor (shr lo 1) (shl (band hi 1) 31))
                          (shr hi 1)))))))

(define (bit-set k lo hi)
  (if (< k 32) (bor lo (shl 1 k)) hi))

(define (bit-set-hi k hi)
  (if (< k 32) hi (bor hi (shl 1 (- k 32)))))

(define (bit-get k lo hi)
  (if (< k 32) (band lo (shl 1 k)) (band hi (shl 1 (- k 32)))))

(define (consume-nonce n)
  (nonce-window-check n)
  (let ((k (- n (get-num "ononce")))
        (lo (get-num "obm_lo"))
        (hi (get-num "obm_hi")))
    (let ((cur (bit-get k lo hi)))
      (if (!= cur 0)
          (die "ERR_NONCE_ALREADY_USED")
          (let ((_ (near/storage_set "obm_lo"
                      (to-string (bit-set k lo hi)))))
            (let ((_ (near/storage_set "obm_hi"
                        (to-string (bit-set-hi k hi)))))
              (slide-window (bit-set k lo hi)
                            (bit-set-hi k hi))))))))


;; ── auth ─────────────────────────────────────────────────────────

;; (verify-owner action sig expires nonce) — aborts on any failure.
;; Wire format (Rust parity):
;;   "expires {exp}.000000000: {action} | nonce: {n} | contract: {id}"
;;   BIP-340 over SHA256(msg), hex pk + hex sig.
(define (verify-owner action sig expires nonce)
  (let ((ts (near/block_timestamp)))
    (if (u128/gt ts expires)
        (die "ERR_SIG_EXPIRED")
        0)
    (let ((msg (str-cat
                 (str-cat
                   (str-cat
                     (str-cat
                       (str-cat (str-cat "expires " expires)
                                ".000000000: ")
                       action)
                     " | nonce: ")
                   nonce)
                 (str-cat " | contract: " (near/current_account_id))))
          (pk (get-str "owner_npub0")))
      (if (= (str-length pk) 0)
          (die "ERR_NOT_INITIALIZED")
          0)
      (let ((ok (schnorr-verify (hex-decode pk)
                                (hex-decode sig)
                                (sha256-hash msg))))
        (if (= ok 1)
            (consume-nonce (str->num nonce))
            (die "ERR_INVALID_OWNER_SIGNATURE"))))))

;; pause has NO nonce slot (Rust: owner-or-guardian, no consume_nonce)
(define (pause)
  (let ((sig (near/json_get_str "signature"))
        (expires (near/json_get_str "expires_at"))
        (ts (near/block_timestamp)))
    (if (u128/gt ts expires)
        (die "ERR_SIG_EXPIRED")
        0)
    (let ((msg (str-cat
                 (str-cat (str-cat "expires " expires)
                          ".000000000: pause")
                 (str-cat " | contract: " (near/current_account_id))))
          (pk (get-str "owner_npub0")))
      (if (= (str-length pk) 0)
          (die "ERR_NOT_INITIALIZED")
          0)
      (let ((ok (schnorr-verify (hex-decode pk)
                                (hex-decode sig)
                                (sha256-hash msg))))
        (if (= ok 1)
            0
            (die "ERR_NOT_AUTHORIZED_TO_PAUSE"))
        (near/storage_set "paused" "1")))))

;; ── name validation ──────────────────────────────────────────────

;; LANDMINE: str-index-of requires a literal needle at emit time —
;; so char membership is a scan over NAME_CHARS via str-slice equality.
(define (char-matches a b j m)
  (if (= j m)
      0
      (if (= a (str-slice b j (+ j 1)))
          1
          (char-matches a b (+ j 1) m))))

(define (name-char-ok s i)
  (let ((c (str-slice s i (+ i 1))))
    (char-matches c NAME_CHARS 0 (str-length NAME_CHARS))))

(define (name-valid-loop s i n)
  (if (>= i n)
      1
      (if (= (name-char-ok s i) 1)
          (name-valid-loop s (+ i 1) n)
          0)))

(define (name-valid s)
  (let ((n (str-length s)))
    (if (= n 0)
        0
        (if (> n 64)
            0
            (name-valid-loop s 0 n)))))

;; ── init: one-time owner npub (x-only, 64 hex chars) ─────────────

(define (init)
  (if (!= (str-length (get-str "owner_npub0")) 0)
      (die "ERR_ALREADY_INITIALIZED")
      0)
  (let ((npub (near/json_get_str "npub")))
    (if (!= (str-length npub) 64)
        (die "ERR_BAD_NPUB")
        0)
    (near/storage_set "owner_npub0" npub))
  0)

;; ── create_wallet ─────────────────────────────────────────────────
;; Rust: assert_not_paused, verify_owner("create_wallet:{name}"),
;; deposit ≥ 0.5N, name checks, wallet insert + event.

(define (create_wallet)
  (let ((name (near/json_get_str "name"))
        (sig (near/json_get_str "signature"))
        (expires (near/json_get_str "expires_at"))
        (nonce (near/json_get_str "nonce")))
    (if (= (str-length expires) 0)
        (die "ERR_ARG_EXPIRES")
        0)
    (if (= (str-length nonce) 0)
        (die "ERR_ARG_NONCE")
        0)
    (if (!= (str-length (get-str "paused")) 0)
        (die "ERR_PAUSED")
        0)
    (verify-owner (str-cat "create_wallet:" name) sig expires nonce))
  (let ((name (near/json_get_str "name")))
    (if (near/deposit-gte 1001882102603448320 27105)
        0
        (die "ERR_STORAGE_DEPOSIT"))
    (if (!= (str-length (get-str (str-cat "w:" name))) 0)
        (die "ERR_WALLET_EXISTS")
        0)
    (if (= (name-valid name) 0)
        (die "ERR_NAME_INVALID_CHARS")
        0)
    (near/storage_set (str-cat "w:" name)
      (str-cat
        (str-cat
          (str-cat (str-cat "{\"name\":\"" name) "\",\"creator\":\"")
          (near/predecessor_account_id))
        (str-cat
          (str-cat (str-cat "\",\"created_at\":\"" (near/block_timestamp))
                   (str-cat "\",\"deposit\":\"" (near/attached_deposit_u128)))
          "\"}"))))
  (near/log (str-cat "wallet created: " (near/json_get_str "name")))
  0)

;; ── unpause ────────────────────────────────────────────────────────

(define (unpause)
  (let ((sig (near/json_get_str "signature"))
        (expires (near/json_get_str "expires_at"))
        (nonce (near/json_get_str "nonce")))
    (if (= (str-length expires) 0)
        (die "ERR_ARG_EXPIRES")
        0)
    (if (= (str-length nonce) 0)
        (die "ERR_ARG_NONCE")
        0)
    (verify-owner "unpause" sig expires nonce))
  (near/storage_remove "paused")
  0)

;; ── views ─────────────────────────────────────────────────────────

(define (get_wallet)
  (let ((name (near/json_get_str "name")))
    (near/json_return_str (get-str (str-cat "w:" name)))))

(define (get_owner_nonce)
  (near/json_return_str (num-str "ononce")))

(define (is_paused)
  (near/json_return_str (num-str "paused")))

(define (get_version)
  (near/json_return_str VERSION))

;; ── public self-test (mirrors Rust test_verify_nostr) ────────────

(define (test_verify_nostr)
  (let ((msg (near/json_get_str "message"))
        (pk (near/json_get_str "pubkey_hex"))
        (sig (near/json_get_str "signature")))
    (let ((ok (schnorr-verify (hex-decode pk)
                              (hex-decode sig)
                              (sha256-hash msg))))
      (if (= ok 1)
          0
          (die "Invalid schnorr signature: verification failed"))
      (near/json_return_str "true"))))

;; ── exports ───────────────────────────────────────────────────────

;; BENCH-ONLY: raw nonce consumption probe (remove before deploy)
(define (dbg_nonce)
  (let ((n (str->num (near/json_get_str "n"))))
    (let ((r (consume-nonce n)))
      (near/json_return_str (to-string r)))))

(define (dbg_state)
  (near/json_return_str
    (str-cat (str-cat "ononce=" (num-str "ononce"))
             (str-cat (str-cat " lo=" (num-str "obm_lo"))
                      (str-cat " hi=" (num-str "obm_hi"))))))

(export "init" init #f)
;; BENCH-ONLY: post-auth half of create_wallet (no signature)
(define (dbg_cw)
  (let ((name (near/json_get_str "name")))
    (if (near/deposit-gte 1001882102603448320 27105)
        0
        (die "ERR_STORAGE_DEPOSIT"))
    (if (!= (str-length (get-str (str-cat "w:" name))) 0)
        (die "ERR_WALLET_EXISTS")
        0)
    (if (= (name-valid name) 0)
        (die "ERR_NAME_INVALID_CHARS")
        0)
    (near/storage_set (str-cat "w:" name)
      (str-cat
        (str-cat
          (str-cat (str-cat "{\"name\":\"" name) "\",\"creator\":\"\"")
          (near/predecessor_account_id))
        (str-cat
          (str-cat (str-cat "\",\"created_at\":\"\"" (near/block_timestamp))
                   (str-cat "\",\"deposit\":\"\"" (near/attached_deposit_u128)))
          "\"}"))))
  (near/log (str-cat "wallet created: " (near/json_get_str "name")))
  0)

(export "dbg_nonce" dbg_nonce #f)
(export "dbg_cw" dbg_cw #f)
(export "dbg_state" dbg_state #t)
(export "create_wallet" create_wallet #f)
(export "pause" pause #f)
(export "unpause" unpause #f)
(export "test_verify_nostr" test_verify_nostr #t)
(export "get_wallet" get_wallet #t)
(export "get_owner_nonce" get_owner_nonce #t)
(export "is_paused" is_paused #t)
(export "get_version" get_version #t)
