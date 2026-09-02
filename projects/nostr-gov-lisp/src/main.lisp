;;;
;;; nostr-gov-lisp — Phase 1
;;;
;;; Port of Kampouse/nostr-gov (Rust/near-sdk) to lisp-rlm.
;;; Nostr-keyed wallet creation with BIP-340 schnorr auth, sliding
;;; nonce window, pause/unpause, and public schnorr self-test.
;;;
;;; WIRE COMPATIBILITY (signatures valid in the Rust original are
;;; valid here — differential-tested):
;;;   owner msg  = 'expires {exp}.000000000: {action} | nonce: {n} | contract: {id}'
;;;   pause msg  = 'expires {exp}.000000000: pause | contract: {id}'   (no nonce!)
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
;;;   - guarded str->num (never '' into arithmetic)
;;;   - owner bitmap is u64 → split into lo(b0..31)/hi(b32..63) pairs;
;;;     each half ≤ 2^32-1, far inside the 61-bit tagged payload.
;;;   - guard pattern: (if bad (die '..') 0) as a STATEMENT,
;;;     side effects after — never abort mixed into value branches
;;;     (near/abort is int-typed; storage_set/json_return are nil).
;;;

(define VERSION "1")
(define EMPTY "")

;; 0.5 NEAR storage deposit = 500000000000000000000000 yocto
;;   = hi<<64 | lo  → (near/deposit-gte 1001882102603448320 27105)
(define DEP_LO 1001882102603448320)
(define DEP_HI 27105)

;;;
;;; Phase 1.5: EVENT AUTH (NIP-46-compatible, kind 37500)
;;;
;;; Problem: nostr signers (NIP-07 extensions, NIP-46 bunkers) sign
;;; EVENTS only — never arbitrary clear-sign strings. The webapp
;;; therefore had to hold the raw secret key (schnorrSign in-browser).
;;;
;;; Fix: owner ops now ALSO authenticate via a governance event:
;;;   id  = SHA256(compact-json([0,pk,created_at,kind,tags,content]))
;;;   sig = BIP-340(pk, sk, id)          (standard nostr event sig)
;;; Contract reconstructs the serialization from the arg fields and
;;; verifies sig over sha256(serialized) — equivalent to the id-binding
;;; + sig check of Rust verify_event (nostr_event.rs), stronger than
;;; comparing claimed ids: no string-equality on 64-hex needed.
;;;
;;; Required tags (charset: no ''', no '\', values quote-free):
;;;   ['action','create_wallet:<name>']  ['nonce','<n>']
;;;   ['expires','<ns>']                 ['contract','<account>']
;;; content is free-form (signed, not parsed).
;;;
;;; The legacy clear-sign path REMAINS (bots/API clients); dispatch on
;;; args: non-empty 'event_id_hex' ⇒ event path, else legacy.

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

(define (get-str k) (default (near/storage_get k) ""))

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
;;   'expires {exp}.000000000: {action} | nonce: {n} | contract: {id}'
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
      ;; NOTE: hex/sha/schnorr args are de-nested into bindings — nested
      ;; inline calls hit emitter scratch-local aliasing (see GAPS.md)
      (let ((pkb (hex-decode pk))
            (sigb (hex-decode sig))
            (mh (hex-decode (sha256-hash msg))))
        (let ((ok (schnorr-verify pkb sigb mh)))
          (if (= ok 1)
              (consume-nonce (str->num nonce))
              (die "ERR_INVALID_OWNER_SIGNATURE")))))))

;; pause has NO nonce slot (Rust: owner-or-guardian, no consume_nonce)
(define (pause-legacy)
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
                                (hex-decode (sha256-hash msg)))))
        (if (= ok 1)
            0
            (die "ERR_NOT_AUTHORIZED_TO_PAUSE"))
        (near/storage_set "paused" "1")))))

(define (pause)
  (if (= (str-length (near/json_get_str "ev")) 0)
      (pause-legacy)
      (let ((ok (verify-guardian-event "pause")))
        (near/storage_set "paused" "1"))))

;; ── Phase 1.5: event auth ────────────────────────────────────────

;; tag parsing with LITERAL needles only (landmine: str-index-of /
;; str-contains needles must be source literals). index-of miss = -1.
;; tag-rest: substring after the literal, EMPTY if absent.
;; tag extractors — LITERAL needles inlined per landmine (index-of
(memory 48)

(define (tag-action tags)
  (let ((i (str-index-of tags "[\"action\",\"")))
    (if (= i -1)
        EMPTY
        (let ((rest (str-slice tags (+ i (str-length "[\"action\",\"")) (str-length tags))))
          (if (= (str-length rest) 0)
              EMPTY
              (str-slice rest 0 (str-index-of rest "\"")))))))

(define (tag-contract tags)
  (let ((i (str-index-of tags "[\"contract\",\"")))
    (if (= i -1)
        EMPTY
        (let ((rest (str-slice tags (+ i (str-length "[\"contract\",\"")) (str-length tags))))
          (if (= (str-length rest) 0)
              EMPTY
              (str-slice rest 0 (str-index-of rest "\"")))))))

(define (tag-nonce tags)
  (let ((i (str-index-of tags "[\"nonce\",\"")))
    (if (= i -1)
        EMPTY
        (let ((rest (str-slice tags (+ i (str-length "[\"nonce\",\"")) (str-length tags))))
          (if (= (str-length rest) 0)
              EMPTY
              (str-slice rest 0 (str-index-of rest "\"")))))))

(define (tag-expires tags)
  (let ((i (str-index-of tags "[\"expires\",\"")))
    (if (= i -1)
        EMPTY
        (let ((rest (str-slice tags (+ i (str-length "[\"expires\",\"")) (str-length tags))))
          (if (= (str-length rest) 0)
              EMPTY
              (str-slice rest 0 (str-index-of rest "\"")))))))

(define (event-serialize pk cat kind tags content)
  (str-cat "[0,\""
    (str-cat pk (str-cat "\","
      (str-cat cat (str-cat ","
        (str-cat kind (str-cat ","
          (str-cat tags (str-cat ",\""
            (str-cat content "\"]")))))))))))

(define (verify-owner-event action-str)
  (let ((pk (near/json_get_str "pk"))
        (kind (near/json_get_str "kind"))
        (tags (near/json_get_str "tags"))
        (content (near/json_get_str "ct"))
        (sig (near/json_get_str "sig"))
        (cat (near/json_get_str "cat")))
    (if (!= (str-length pk) 64)
        (die "ERR_EVENT_PK_LEN")
        0)
    (if (!= (str-length sig) 128)
        (die "ERR_EVENT_SIG_LEN")
        0)
    (if (!= kind "37500")
        (die "ERR_EVENT_KIND")
        0)
    (if (= pk (get-str "owner_npub0"))
        0
        (die "ERR_EVENT_PK_MISMATCH"))
    (let ((ta (tag-action tags))
          (tc (tag-contract tags))
          (tn (tag-nonce tags))
          (te (tag-expires tags)))
      (let ((ts (near/block_timestamp)))
      (if (u128/gt ts te)
          (die "ERR_SIG_EXPIRED")
          0))
      (if (= ta action-str)
          0
          (die "ERR_EVENT_ACTION"))
      (if (= tc (near/current_account_id))
          0
          (die "ERR_EVENT_CONTRACT"))
      (let ((serialized (event-serialize pk cat kind tags content)))
        (let ((ok (schnorr-verify (hex-decode pk)
                                  (hex-decode sig)
                                  (hex-decode (sha256-hash serialized)))))
          (if (= ok 1)
              (consume-nonce (str->num tn))
              (die "ERR_EVENT_SIG_INVALID")))))))

;; guardian variant: pause/unpause carry NO nonce (mirrors legacy)
(define (verify-guardian-event action-str)
  (let ((pk (near/json_get_str "pk"))
        (kind (near/json_get_str "kind"))
        (tags (near/json_get_str "tags"))
        (content (near/json_get_str "ct"))
        (sig (near/json_get_str "sig"))
        (cat (near/json_get_str "cat")))
    (if (!= (str-length pk) 64)
        (die "ERR_EVENT_PK_LEN")
        0)
    (if (!= (str-length sig) 128)
        (die "ERR_EVENT_SIG_LEN")
        0)
    (if (!= kind "37500")
        (die "ERR_EVENT_KIND")
        0)
    (if (= pk (get-str "owner_npub0"))
        0
        (die "ERR_EVENT_PK_MISMATCH"))
    (let ((ta (tag-action tags))
          (tc (tag-contract tags))
          (te (tag-expires tags)))
      (let ((ts (near/block_timestamp)))
      (if (u128/gt ts te)
          (die "ERR_SIG_EXPIRED")
          0))
      (if (= ta action-str)
          0
          (die "ERR_EVENT_ACTION"))
      (if (= tc (near/current_account_id))
          0
          (die "ERR_EVENT_CONTRACT"))
      (let ((serialized (event-serialize pk cat kind tags content)))
        (let ((ok (schnorr-verify (hex-decode pk)
                                  (hex-decode sig)
                                  (hex-decode (sha256-hash serialized)))))
          (if (= ok 1)
              0
              (die "ERR_EVENT_SIG_INVALID")))))))

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
;; Rust: assert_not_paused, verify_owner('create_wallet:{name}'),
;; deposit ≥ 0.5N, name checks, wallet insert + event.

(define (create_wallet)
  (let ((name (near/json_get_str "name"))
        (sig (near/json_get_str "signature"))
        (expires (near/json_get_str "expires_at"))
        (nonce (near/json_get_str "nonce")))
    ;; ev routing first (mirrors the Rust reference): event-auth calls carry
    ;; tags, not legacy expires_at/nonce, so arg validation is legacy-only.
    (if (= (str-length (near/json_get_str "ev")) 0)
      (begin
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
      (verify-owner-event (str-cat "create_wallet:" name))))
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
    (if (= (str-length (near/json_get_str "ev")) 0)
        (begin
          (if (= (str-length expires) 0)
              (die "ERR_ARG_EXPIRES")
              0)
          (if (= (str-length nonce) 0)
              (die "ERR_ARG_NONCE")
              0)
          (verify-owner "unpause" sig expires nonce))
        (verify-owner-event "unpause")))
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
        (sig (near/json_get_str "sig_hex")))
    (let ((ok (schnorr-verify (hex-decode pk)
                              (hex-decode sig)
                              (hex-decode (sha256-hash msg)))))
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

;; BENCH-ONLY: probe json_get_str absent-key semantics

;; BENCH-ONLY: index-returning probes (discriminate needle vs hay corruption)

;; BENCH-ONLY: does the SCAN see backslashes the slice doesn't?
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

;; BENCH-ONLY: tag extractor probes


;; ── Phase 2: proposals ─────────────────────────────────────────────
(define (count-commas s)
  (count-commas-loop s 0 (str-length s) 0))
(define (count-commas-loop s i n c)
  (if (>= i n)
      c
      (if (= (str-slice s i (+ i 1)) ",")
          (count-commas-loop s (+ i 1) n (+ c 1))
          (count-commas-loop s (+ i 1) n c))))
(define (approver-count pks)
  (+ (count-commas pks) 1))
(define (nth-field s k)
  (nth-field-loop s k 0 (str-length s) 0))
(define (nth-field-loop s k i n start)
  (if (>= i n)
      (if (= k 0) (str-slice s start n) "")
      (if (= (str-slice s i (+ i 1)) ",")
          (if (= k 0) (str-slice s start i) (nth-field-loop s (- k 1) (+ i 1) n (+ i 1)))
          (nth-field-loop s k (+ i 1) n start))))
(define (pow2 k)
  (pow2-loop k "1"))
(define (pow2-loop k acc)
  (if (= k 0)
      acc
      (pow2-loop (- k 1) (u128/mul acc "2"))))
(define (bit-set? bm k)
  (= (u128/mod (u128/div bm (pow2 k)) "2") "1"))
(define (set-bit bm k)
  (u128/add bm (pow2 k)))
(define (auth-owner action)
  (let ((sig (near/json_get_str "signature"))
        (expires (near/json_get_str "expires_at"))
        (nonce (near/json_get_str "nonce")))
    (if (= (str-length (get-str "paused")) 0)
        0
        (die "ERR_PAUSED"))
    (if (= (str-length (near/json_get_str "ev")) 0)
        ;; legacy sig path: expires/nonce are top-level args
        (if (= (str-length expires) 0)
            (die "ERR_ARG_EXPIRES")
            (if (= (str-length nonce) 0)
                (die "ERR_ARG_NONCE")
                (verify-owner action sig expires nonce)))
        ;; event-auth path: expires/nonce live in event tags
        (verify-owner-event action))))
(define (set_approvers)
  (let ((name (near/json_get_str "name")))
    (if (= (str-length (get-str (str "w:" name))) 0)
        (die "ERR_WALLET_NOT_FOUND")
        0)
    (auth-owner (str "set_approvers:" name)))
  (let ((name (near/json_get_str "name"))
        (pks (near/json_get_str "pks"))
        (thr (near/json_get_str "thr")))
    (if (= (str-length pks) 0)
        (die "ERR_APPROVERS_EMPTY")
        0)
    (if (or (= (str->num thr) 0)
            (> (str->num thr) (approver-count pks)))
        (die "ERR_THRESHOLD_INVALID")
        0)
    (near/storage_set (str "a:" name)
                      (str "{\"thr\":\"" thr "\",\"pks\":\"" pks "\"}"))
    (near/log (str "approvers set: " name))
    0))
(export "set_approvers" set_approvers #f)
(define (propose)
  (let ((name (near/json_get_str "name")))
    (if (= (str-length (get-str (str "w:" name))) 0)
        (die "ERR_WALLET_NOT_FOUND")
        0)
    (let ((id (if (= (str-length (get-str (str "pi:" name))) 0)
                  "0"
                  (get-str (str "pi:" name)))))
      (auth-owner (str "propose:" name ":" id))))
  (let ((name (near/json_get_str "name"))
        (pexp (near/json_get_str "pexp"))
        (amt (near/json_get_str "am"))
        (to (near/json_get_str "rc"))
        (ts (near/block_timestamp)))
    (let ((id (if (= (str-length (get-str (str "pi:" name))) 0)
                  "0"
                  (get-str (str "pi:" name)))))
      (if (= (str-length to) 0)
          (die "ERR_MISSING_RECIPIENT")
          0)
      (if (= (str-length amt) 0)
          (die "ERR_MISSING_AMOUNT")
          0)
      (if (u128/lt pexp (to-string ts))
          (die "ERR_EXPIRED")
          0)
      ;; nil-guard: json_get_str(missing) is nil — default to "" so a
      ;; tk-less proposal stores "" (NEAR payout) instead of routing the
      ;; execute to an FT promise on account "nil" (live-caught 2026-09-02)
      (let ((tk (default (near/json_get_str "tk") "")))
        (near/storage_set (str "p:" name ":" id)
                          (str "{\"id\":\"" id "\",\"st\":\"active\",\"exp\":\"" pexp
                               "\",\"amt\":\"" amt "\",\"to\":\"" to
                               "\",\"tk\":\"" tk
                               "\",\"bl\":\"0\",\"bh\":\"0\",\"ac\":\"0\"}")))
      (near/storage_set (str "pi:" name)
                        (to-string (+ (str->num id) 1)))
      (near/log (str "proposal " id " created for " name))
      0)))
(export "propose" propose #f)
(define (approve)
  (let ((name (near/json_get_str "name"))
        (id (near/json_get_str "id"))
        (ix (near/json_get_str "ix"))
        (pk (near/json_get_str "pubkey_hex"))
        (sig (near/json_get_str "signature"))
        (exp (near/json_get_str "expires_at"))
        (ts (near/block_timestamp)))
    (let ((p (get-str (str "p:" name ":" id)))
          (a (get-str (str "a:" name))))
      (if (= (str-length p) 0)
          (die "ERR_PROPOSAL_NOT_FOUND")
          0)
      (if (= (str-length a) 0)
          (die "ERR_APPROVERS_NOT_SET")
          0)
      (if (= (str-length pk) 64)
          0
          (die "ERR_APPROVER_PK_LEN"))
      (if (= (str-length sig) 128)
          0
          (die "ERR_APPROVER_SIG_LEN"))
      (let ((st (json-get-str "st" p))
            (pexp (json-get-str "exp" p))
            (bl (json-get-str "bl" p))
            (ac (json-get-str "ac" p))
            (amt (json-get-str "amt" p))
            (to (json-get-str "to" p))
            (pks (json-get-str "pks" a))
            (thr (json-get-str "thr" a)))
        (if (= st "active")
            0
            (die "ERR_NOT_ACTIVE"))
        (if (u128/lt pexp (to-string ts))
            (die "ERR_PROPOSAL_EXPIRED")
            0)
        (if (u128/lt exp (to-string ts))
            (die "ERR_SIG_EXPIRED")
            0)
        (if (< (str->num ix) (approver-count pks))
            0
            (die "ERR_INVALID_APPROVER_INDEX"))
        (if (= (nth-field pks (str->num ix)) pk)
            0
            (die "ERR_APPROVER_PK_MISMATCH"))
        (if (= (schnorr-verify (hex-decode pk)
                               (hex-decode sig)
                               (hex-decode (sha256-hash (str "expires " exp ".000000000: approve:"
                                                 name ":" id ":" ix
                                                 " | contract: " (near/current_account_id)))))
             1)
            0
            (die "ERR_APPROVER_SIG_INVALID"))
        (if (bit-set? bl (str->num ix))
            (die "ERR_ALREADY_APPROVED")
            0)
        (let ((nac (+ (str->num ac) 1))
              (nbl (set-bit bl (str->num ix)))
              (nsth (if (>= (+ (str->num ac) 1) (str->num thr))
                        "approved"
                        "active")))
          (near/storage_set (str "p:" name ":" id)
                            (str "{\"id\":\"" id "\",\"st\":\"" nsth
                                 "\",\"exp\":\"" pexp "\",\"amt\":\"" amt
                                 "\",\"to\":\"" to "\",\"tk\":\"" (json-get-str "tk" p)
                                 "\",\"bl\":\"" nbl
                                 "\",\"bh\":\"0\",\"ac\":\"" (to-string nac) "\"}"))
          (near/log (str "approval " ix " on " name ":" id))
          0)))))
(export "approve" approve #f)
(define (execute)
  (let ((name (near/json_get_str "name"))
        (id (near/json_get_str "id"))
        (ts (near/block_timestamp)))
    (let ((p (get-str (str "p:" name ":" id))))
      (if (= (str-length p) 0)
          (die "ERR_PROPOSAL_NOT_FOUND")
          0)
      (auth-owner (str "execute:" name ":" id))
      (if (= (json-get-str "st" p) "approved")
          0
          (die "ERR_NOT_APPROVED"))
      (if (u128/lt (json-get-str "exp" p) (to-string ts))
          (die "ERR_PROPOSAL_EXPIRED")
          0)
      ;; NOTE (2026-09-02): the FT branch below is DISABLED in the lisp twin —
      ;; emitter bug: host-call arms inside non-tail ifs compile to dead code
      ;; (probes 9–14, GAPS.md "statement-if host-arm dead-branch"). The TS
      ;; twin emits the same branch correctly (live-proven on testnet). Once
      ;; the emitter is fixed, restore:
      ;;   (if (= (str-length (json-get-str "tk" p)) 0)
      ;;       (near/transfer_u128 (json-get-str "to" p) (json-get-str "amt" p))
      ;;       (let ((pi (near/promise_batch_create (json-get-str "tk" p))))
      ;;         (near/promise_batch_action_function_call pi "ft_transfer"
      ;;           (str "{\"receiver_id\":\"" (json-get-str "to" p)
      ;;                "\",\"amount\":\"" (json-get-str "amt" p)
      ;;                "\",\"memo\":\"nostr-gov\"}")
      ;;           "1" 5000000000000)))
      (near/transfer_u128 (json-get-str "to" p)
                          (json-get-str "amt" p))
      (near/storage_set (str "p:" name ":" id)
                        (str "{\"id\":\"" id "\",\"st\":\"executed\",\"exp\":\""
                             (json-get-str "exp" p) "\",\"amt\":\"" (json-get-str "amt" p)
                             "\",\"to\":\"" (json-get-str "to" p)
                             "\",\"tk\":\"" (json-get-str "tk" p)
                             "\",\"bl\":\"" (json-get-str "bl" p)
                             "\",\"bh\":\"0\",\"ac\":\""
                             (json-get-str "ac" p) "\"}"))
      (near/log (str "proposal " id " executed: " name))
      0)))
(export "execute" execute #f)
(define (get_proposal)
  (near/json_return_str (get-str (str "p:" (near/json_get_str "name")
                                      ":" (near/json_get_str "id")))))
(export "get_proposal" get_proposal #t)
(define (get_approvers)
  (near/json_return_str (get-str (str "a:" (near/json_get_str "name")))))
(export "get_approvers" get_approvers #t)
