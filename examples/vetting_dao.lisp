;; ── Vetting DAO — peer-vouched membership on NEAR ─────────────────────────
;; Target: lisp-rlm near target (compile --target near)
;;
;; Design: no single admin gate. Already-vetted members vouch for pending
;; candidates; at `threshold` distinct vouches the candidate is auto-promoted.
;; Bootstrap: init() seeds the first members via admin_add + renounceable key.
;;
;; Storage layout (8-byte tagged-int values; keys are tagged strings):
;;   v:thr                      current vouch threshold
;;   v:cnt                      number of vetted members
;;   v:status:<account>         1 = applied/pending, 2 = vetted member
;;   v:count:<account>          vouch count for candidate <account>
;;   v:vo:<voter>:on:<cand>     marker: <voter> already vouched <cand>
;;   v:admin:<account>          admin key (bootstrap backstop, renounceable)
;;
;; Toolchain constraints honored here (lisp-rlm 0.1.16, near target):
;;   - str_cat is 2-arg ONLY in NEAR target; never nest it inside itself
;;   - str_cat must not appear in view exports (frame allocator) — views
;;     compose keys with near/kv-get instead (probe-verified view-safe)
;;   - vouch markers use near/kv (composite keys, no str_cat), value 9
;;   - store/load round-trip i64 values as-is; = is structural; near/load
;;     miss → nil; json_get_str miss → nil
;;   - mock: near-mock call <state> dao=<wasm> dao <m> [args] --signer/--attach

;; ── key builders ──
(define (st-k acct)   (str_cat "v:status:" acct))
(define (cnt-k acct)  (str_cat "v:count:" acct))
(define (adm-k a)     (str_cat "v:admin:" a))

;; membership = status EXACTLY 2. has_key alone would let pending applicants
;; (status 1) self-vouch — caught by the intent suite, do not regress.
(define (is-member a) (= (near/load (st-k a)) 2))
(define (is-admin a)  (near/has_key (adm-k a)))

;; ── init: threshold + genesis admin (backstop only) ──
(define (init)
  (begin
    (near/store "v:thr" (near/json_get_int "threshold"))
    (near/store "v:cnt" 0)
    (near/store (adm-k (near/json_get_str "admin")) 1)))

;; ── apply: candidate stakes a deposit to enter the pending pool ──
(define (apply)
  (begin
    (let ((c (near/json_get_str "candidate")))
      (if (near/has_key (st-k c))
        (near/panic "already known")
        (if (> (near/attached_deposit) 0)
          (near/store (st-k c) 1)
          (near/panic "need deposit"))))))

;; ── vouch: a vetted member vouches for a pending candidate ──
(define (vouch)
  (begin
    (let ((v (near/predecessor_account_id))
          (c (near/json_get_str "candidate")))
      (if (is-member v)
        (if (= (near/load (st-k c)) 1)
          (if (near/kv-get "v:vo:" v ":on:" c)
            (near/panic "already vouched")
            (begin
              (near/kv "v:vo:" v ":on:" c 9)
              (let ((n (+ (near/load (cnt-k c)) 1)))
                (if (>= n (near/load "v:thr"))
                  (begin
                    (near/store (st-k c) 2)
                    (near/store "v:cnt" (+ (near/load "v:cnt") 1)))
                  (near/store (cnt-k c) n)))))
          (near/panic "not pending"))
        (near/panic "voter not vetted")))))

;; ── promote: manual early promotion once quorum logically met ──
(define (promote)
  (begin
    (let ((c (near/json_get_str "candidate")))
      (if (= (near/load (st-k c)) 1)
        (if (>= (near/load (cnt-k c)) (near/load "v:thr"))
          (near/store (st-k c) 2)
          (near/panic "quorum not met"))
        (near/panic "no application")))))

;; ── admin backstop: seed members (genesis/bootstrap), renounce (decentralize).
;; Admin adds ANY account as a vetted member — how genesis members enter when
;; the set is empty. After renounce_admin there is no admin. ──
(define (admin_add)
  (begin
    (let ((p (near/predecessor_account_id))
          (c (near/json_get_str "candidate")))
      (if (is-admin p)
        (if (= (near/load (st-k c)) 2)
          (near/panic "already vetted")
          (begin
            (near/store (st-k c) 2)
            (near/store "v:cnt" (+ 1 (near/load "v:cnt")))))
        (near/panic "not admin")))))

(define (renounce_admin)
  (let ((p (near/predecessor_account_id)))
    (if (is-admin p)
      (near/remove (adm-k p))
      (near/panic "not admin"))))

;; ── views (no str_cat — kv-get only) ──
(define (get_status)       (near/kv-get "v:status:" (near/json_get_str "candidate")))
(define (get_threshold)    (near/load "v:thr"))
(define (get_member_count) (near/load "v:cnt"))

(export "init" init false)
(export "apply" apply false)
(export "vouch" vouch false)
(export "promote" promote false)
(export "admin_add" admin_add false)
(export "renounce_admin" renounce_admin false)
(export "get_status" get_status true)
(export "get_threshold" get_threshold true)
(export "get_member_count" get_member_count true)
