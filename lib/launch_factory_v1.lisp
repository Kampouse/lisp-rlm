;; ═══════════════════════════════════════════════════════════════════════
;; lib/launch_factory_v1.lisp — vesting launch factory (2026-10-20)
;;
;; Meme-launchpad custody contract: holds the FOUNDING CLMM position and
;; vests it back to the creator on a hours-scale schedule. The meme
;; reality (dead in ≤1 week) sets the constants: cliff ≥ 1h, vest ≤ 7d.
;;
;; Lifecycle:
;;   1. init {tok,qt,pool}  — one-shot id pin (factory per pool).
;;   2. reg                 — self-register on the pool (claim gate).
;;   3. Creator seeds via TWO ft_transfer_call legs (the pool's mint
;;      protocol, factory as sender ⇒ position owner = factory):
;;        leg X: tok.ft_transfer_call → factory, msg
;;          {"a":"seed","lo":a,"hi":b,"d":d,"cliff":ns,"vest":ns}
;;        leg Y: same msg via the QUOTE token — completes the mint.
;;      The factory forwards the ENTIRE deposit to the pool; C3
;;      over-attach refunds the req-excess to the factory. Both legs done
;;      ⇒ PH=3, T0 = now+cliff, T1 = T0+vest. Leg-2 msg must repeat the
;;      exact lo/hi/d strings (byte check — the pool enforces the same).
;;   4. wd {d} — creator-only tranche burn: allowance =
;;      D·(now−T0)/(T1−T0) floor (u128-muldiv, string domain end to end).
;;      Burns via pool "burn" (burnc: folds fees of the tranche too) and
;;      claims BOTH sides, forwarding backing+fees to the creator.
;;   5. sweep — creator-only retry: re-claims + forwards any pending
;;      CX/CY from a wd that died mid-chain (no burn — no over-burn).
;;   6. state — packed view incl. allowance at the current timestamp.
;;
;; Fees: burnc's feefold credits the tranche's fees alongside its backing
;; (fees ride withdrawals; a fully-vested final wd sweeps everything).
;; No fold-only collect exists in v5 (collect == claim).
;;
;; Failure model (pool C2 patterns):
;;   - seed leg abort (ERR_SHORT/RANGE/MISMATCH): token resolve refunds
;;     the factory ⇒ onleg refunds the creator in full; PH rewinds (leg 2
;;     failure keeps leg 1 pending — creator retries Y or pool "cancel").
;;   - burn abort: onburn restores BUR then aborts (retry later).
;;   - claim abort: owed stays pool-side untouched; creator sweeps.
;;
;; Time discipline (timelock_vault_v1 ABI pin): block_timestamp is an
;; EXACT DECIMAL STRING of ns — compared with li-cmp, arithmetics with
;; li-add/li-sub, NEVER str->num (ns ≈ 1.8e18 exceeds the 60-bit box).
;;
;; Call discipline: one value-producing call per let binding; sequential
;; single-binding lets; `begin` wraps only effect-only sequences. All
;; money/timestamp math is li-* / u128-muldiv (string domain).
;; ═══════════════════════════════════════════════════════════════════════

(define (lf-bz k) (let ((v (near/load-bytes k))) (if (nil? v) "0" v)))
(define (lf-put k v) (near/store-bytes k v))
(define (lf-fail m) (near/abort m))

;; ── bond: all remaining prepaid gas minus a reserve. A receipt can only
;; promise gas it still holds, so every hop forwards dynamically instead
;; of hardcoding Tgas literals (the gas pond drains hop by hop).
(define (lf-bond r)
  (- (near/prepaid_gas) (near/used_gas) r))

;; li helpers (limb_math string ops — same 3 as v5/timelock)
(define (li-add a b)
  (let ((A (str->limbs a)) (B (str->limbs b)) (r (buf-alloc 64)))
    (limbs->str r (limb-add (car A) (car B) r (car (cdr A)) (car (cdr B))))))
(define (li-sub a b)
  (let ((A (str->limbs a)) (B (str->limbs b)) (r (buf-alloc 64)))
    (limbs->str r (limb-sub (car A) (car B) r (car (cdr A)) (car (cdr B))))))
(define (li-cmp a b)
  (let ((A (str->limbs a)) (B (str->limbs b)))
    (limb-cmp (car A) (car B) (car (cdr A)) (car (cdr B)))))

;; cliff/vest bounds: 1h .. 7d (ns strings)
(define LF-MIN "3600000000000")
(define LF-MAX "604800000000000")

;; ── string splitting: "burnc-ok:<cx>:<cy>" ──────────────────────────────
;; prefix is constant; take the rest, then split at the FIRST ":".
(define (lf-rest s)
  (str-slice s 9 (str-len s)))
(define (lf-fst res)
  (loop ((i 0))
    (if (str= (str-slice res i (+ i 1)) ":")
        (str-slice res 0 i)
        (recur (+ i 1)))))
(define (lf-snd res)
  (let ((k (str-len res)))
    (loop ((i (- k 1)))
      (if (str= (str-slice res i (+ i 1)) ":")
          (str-slice res (+ i 1) k)
          (recur (- i 1))))))

;; ── init {tok,qt,pool} — one-shot ───────────────────────────────────────
(define (lf-init)
  (let ((inp (near/input)))
    (if (not (str= (lf-bz "LF|POOL") "0"))
        (lf-fail "ERR_INIT")
        (begin
          (lf-put "LF|TOK" (json-get-str "tok" inp))
          (lf-put "LF|QT" (json-get-str "qt" inp))
          (lf-put "LF|POOL" (json-get-str "pool" inp))
          "init-ok"))))

;; ── reg — self-register on the pool (claim gate) ────────────────────────
(define (lf-reg)
  (let ((p (near/promise_batch_create (lf-bz "LF|POOL"))))
    (begin
      (near/promise_batch_action_function_call p "register"
        (json-set "{}" "who" (near/current_account_id))
        "0" 30000000000000)
      "reg-ok")))

;; ── seed entry (ft_on_transfer) ─────────────────────────────────────────
;; Forward the WHOLE deposit as the pool mint leg for this side; the
;; pool's C3 resolve refunds the req-excess to the factory.
(define (lf-forward msgjson amt tokkey cbargs)
  (let ((p (near/promise_batch_create (lf-bz tokkey))))
    (begin
      (near/promise_batch_action_function_call p "ft_transfer_call"
        (json-set (json-set (json-set "{}"
          "receiver_id" (lf-bz "LF|POOL")) "amount" amt) "msg" msgjson)
        "0" (lf-bond 40000000000000))
      (near/promise_return
        (near/promise_then p (near/current_account_id) "onleg"
          cbargs "0" 10000000000000)))))

(define (lf-seedmsg lo hi d)
  (json-set (json-set (json-set (json-set "{}"
    "a" "mint") "lo" lo) "hi" hi) "d" d))

(define (lf-seed)
  (let ((inp (near/input)))
  (let ((sender (json-get-str "sender_id" inp)))
  (let ((amt (json-get-str "amount" inp)))
  (let ((msg (json-get-str "msg" inp)))
  (let ((a (json-get-str "lo" msg)))
  (let ((b (json-get-str "hi" msg)))
  (let ((d (json-get-str "d" msg)))
    (if (str= (lf-bz "LF|POOL") "0")
        (near/return amt)
        (if (str= (near/predecessor_account_id) (lf-bz "LF|TOK"))
            ;; ── leg X: first side ──
            (if (not (str= (lf-bz "LF|PH") "0"))
                (near/return amt)
                (let ((cl (json-get-str "cliff" msg)))
                (let ((ve (json-get-str "vest" msg)))
                  (if (or (< (li-cmp cl LF-MIN) 0)
                          (> (li-cmp cl LF-MAX) 0)
                          (< (li-cmp ve LF-MIN) 0)
                          (> (li-cmp ve LF-MAX) 0)
                          (< (li-cmp d "0") 1))
                      (near/return amt)
                      (begin
                        (lf-put "LF|CRE" sender)
                        (lf-put "LF|A" a)
                        (lf-put "LF|B" b)
                        (lf-put "LF|D" d)
                        (lf-put "LF|CLI" cl)
                        (lf-put "LF|VES" ve)
                        (lf-put "LF|PH" "1")
                        (lf-forward (lf-seedmsg a b d) amt "LF|TOK"
                          (json-set (json-set (json-set "{}"
                            "k" "1") "r" sender) "m" amt))
                        "0")))))
            (if (str= (near/predecessor_account_id) (lf-bz "LF|QT"))
                ;; ── leg Y: completes (PH=2 = leg X confirmed) ──
                (if (not (and (str= (lf-bz "LF|PH") "2")
                              (str= a (lf-bz "LF|A"))
                              (str= b (lf-bz "LF|B"))
                              (str= d (lf-bz "LF|D"))))
                    (near/return amt)
                    (begin
                      (lf-put "LF|PH" "3")
                      (lf-forward (lf-seedmsg a b d) amt "LF|QT"
                        (json-set (json-set (json-set "{}"
                          "k" "2") "r" sender) "m" amt))
                      "0"))
                (near/return amt))))))))))))

;; ── onleg — callback of a forwarded mint leg ────────────────────────────
;; promise_result empty ⇒ the pool aborted the leg; the token resolve has
;; already refunded the factory ⇒ refund the creator in full.
(define (lf-onleg)
  (if (str= (near/predecessor_account_id) (near/current_account_id))
      (let ((inp (near/input)))
      (let ((k (json-get-str "k" inp)))
      (let ((who (json-get-str "r" inp)))
      (let ((amt (json-get-str "m" inp)))
      (let ((r (near/promise_result 0)))
        (if (< (str-length r) 1)
            (begin
              (if (str= k "2") (lf-put "LF|PH" "1") (lf-put "LF|PH" "0"))
              (let ((p (near/promise_batch_create
                         (lf-bz (if (str= k "1") "LF|TOK" "LF|QT")))))
                (begin
                  (near/promise_batch_action_function_call p "ft_transfer"
                    (json-set (json-set "{}" "receiver_id" who)
                              "amount" amt)
                    "0" (lf-bond 40000000000000))
                  (near/promise_return
                    (near/promise_then p (near/current_account_id) "onref"
                      (json-set "{}" "r" who) "0" 10000000000000))
                  (near/return "0"))))
            (if (str= k "1")
                (begin (lf-put "LF|PH" "2") "0")
                (let ((ts (near/block_timestamp)))
                  (begin
                    (lf-put "LF|PH" "4")
                    (lf-put "LF|T0" (li-add ts (lf-bz "LF|CLI")))
                    (lf-put "LF|T1" (li-add (lf-bz "LF|T0")
                                            (lf-bz "LF|VES")))
                    "0")))))))))
      (lf-fail "ERR_SELF")))

(define (lf-onref)
  (if (str= (near/predecessor_account_id) (near/current_account_id))
      "refund-ok"
      (lf-fail "ERR_SELF")))

;; ── allowance ────────────────────────────────────────────────────────────
;; floor(D·(now−T0)/(T1−T0)); 0 before the cliff.
(define (lf-allow ts)
  (if (< (li-cmp ts (lf-bz "LF|T0")) 0)
      "0"
      (if (> (li-cmp ts (lf-bz "LF|T1")) 0)
          (lf-bz "LF|D")
          (u128-muldiv (lf-bz "LF|D")
                       (li-sub ts (lf-bz "LF|T0"))
                       (li-sub (lf-bz "LF|T1") (lf-bz "LF|T0"))))))

;; ── wd {d} — creator tranche burn ───────────────────────────────────────
(define (lf-wd)
  (let ((inp (near/input)))
  (let ((who (near/predecessor_account_id)))
  (let ((d (json-get-str "d" inp)))
    (if (not (and (str= who (lf-bz "LF|CRE"))
                  (str= (lf-bz "LF|PH") "4")))
        (lf-fail "ERR_AUTH")
        (let ((ts (near/block_timestamp)))
          (if (< (li-cmp ts (lf-bz "LF|T0")) 0)
              (lf-fail "ERR_VEST")
              (let ((newbur (li-add (lf-bz "LF|BUR") d)))
                (if (> (li-cmp newbur (lf-allow ts)) 0)
                    (lf-fail "ERR_VEST")
                    (begin
                      (lf-put "LF|BUR" newbur)
                      (let ((p (near/promise_batch_create
                                 (lf-bz "LF|POOL"))))
                        (begin
                          (near/promise_batch_action_function_call p "burn"
                            (json-set (json-set (json-set "{}"
                              "d" d) "a" (lf-bz "LF|A")) "b" (lf-bz "LF|B"))
                            "0" (lf-bond 40000000000000))
                          (near/promise_return
                            (near/promise_then p (near/current_account_id)
                              "onburn"
                              (json-set (json-set "{}" "r" who) "d" d)
                              "0" 35000000000000))))))))))))))

;; ── onburn — burnc succeeded: owed = backing + tranche fees ─────────────
;; Failure ⇒ restore BUR (the wd receipt that wrote it already committed).
(define (lf-onburn)
  (if (str= (near/predecessor_account_id) (near/current_account_id))
      (let ((inp (near/input)))
      (let ((who (json-get-str "r" inp)))
      (let ((d (json-get-str "d" inp)))
      (let ((r (near/promise_result 0)))
        (if (< (str-length r) 1)
            (begin
              (lf-put "LF|BUR" (li-sub (lf-bz "LF|BUR") d))
              (lf-fail "ERR_BURN"))
            (let ((cx (lf-fst (lf-rest r)))
                  (cy (lf-snd (lf-rest r))))
              (begin
                (lf-put "LF|CX" cx)
                (lf-put "LF|CY" cy)
                (let ((p (near/promise_batch_create (lf-bz "LF|POOL"))))
                  (begin
                    (near/promise_batch_action_function_call p "collect"
                      (json-set "{}" "t" "X") "0" (lf-bond 9000000000000))
                    (near/promise_return
                      (near/promise_then p (near/current_account_id) "oncx"
                        (json-set "{}" "r" who) "0" (lf-bond 10000000000000)))))))))))))
      (lf-fail "ERR_SELF"))

;; ── oncx — claim X paid the factory ⇒ fire claim Y ──────────────────────
(define (lf-oncx)
  (if (str= (near/predecessor_account_id) (near/current_account_id))
      (let ((who (json-get-str "r" (near/input))))
      (let ((r (near/promise_result 0)))
        (if (< (str-length r) 1)
            (lf-fail "ERR_CLX")
            (let ((p (near/promise_batch_create (lf-bz "LF|POOL"))))
              (begin
                (near/promise_batch_action_function_call p "collect"
                  (json-set "{}" "t" "Y") "0" (lf-bond 7000000000000))
                (near/promise_return
                  (near/promise_then p (near/current_account_id) "oncy"
                    (json-set "{}" "r" who) "0" 12000000000000)))))))
      (lf-fail "ERR_SELF")))

;; ── oncy — claim Y paid ⇒ forward backing+fees of both sides to the creator
(define (lf-oncy)
  (if (str= (near/predecessor_account_id) (near/current_account_id))
      (let ((who (json-get-str "r" (near/input))))
      (let ((r (near/promise_result 0)))
        (if (< (str-length r) 1)
            (lf-fail "ERR_CLY")
            (let ((px (near/promise_batch_create (lf-bz "LF|TOK"))))
            (let ((py (near/promise_batch_create (lf-bz "LF|QT"))))
              (begin
                (near/promise_batch_action_function_call px "ft_transfer"
                  (json-set (json-set "{}" "receiver_id" who)
                            "amount" (lf-bz "LF|CX"))
                  "0" (lf-bond 3500000000000))
                (near/promise_batch_action_function_call py "ft_transfer"
                  (json-set (json-set "{}" "receiver_id" who)
                            "amount" (lf-bz "LF|CY"))
                  "0" (lf-bond 2200000000000))
                (lf-put "LF|CX" "0")
                (lf-put "LF|CY" "0")
                (near/promise_return
                  (near/promise_then py (near/current_account_id) "onfin"
                    (json-set "{}" "r" who) "0" 1500000000000))))))))
      (lf-fail "ERR_SELF")))

(define (lf-onfin)
  (if (str= (near/predecessor_account_id) (near/current_account_id))
      "wd-ok"
      (lf-fail "ERR_SELF")))

;; ── sweep — retry a wd that died mid-claim ──────────────────────────────
;; Claims are idempotent pool-side (owed untouched on failure); this
;; re-claims both sides and forwards the recorded pending amounts.
(define (lf-sweep)
  (let ((who (near/predecessor_account_id)))
    (if (not (and (str= who (lf-bz "LF|CRE"))
                  (str= (lf-bz "LF|PH") "4")
                  (> (li-cmp (lf-bz "LF|CX") "0") 0)))
        (lf-fail "ERR_AUTH")
        (let ((p (near/promise_batch_create (lf-bz "LF|POOL"))))
          (begin
            (near/promise_batch_action_function_call p "collect"
              (json-set "{}" "t" "X") "0" (lf-bond 9000000000000))
            (near/promise_return
              (near/promise_then p (near/current_account_id) "oncx"
                (json-set "{}" "r" who) "0" (lf-bond 10000000000000))))))))

;; ── state — packed view (allowance at current ts) ───────────────────────
(define (lf-state)
  (let ((ts (near/block_timestamp)))
    (str-cat "tok:" (str-cat (lf-bz "LF|TOK")
    (str-cat ":qt:" (str-cat (lf-bz "LF|QT")
    (str-cat ":pool:" (str-cat (lf-bz "LF|POOL")
    (str-cat ":cre:" (str-cat (lf-bz "LF|CRE")
    (str-cat ":a:" (str-cat (lf-bz "LF|A")
    (str-cat ":b:" (str-cat (lf-bz "LF|B")
    (str-cat ":d:" (str-cat (lf-bz "LF|D")
    (str-cat ":t0:" (str-cat (lf-bz "LF|T0")
    (str-cat ":t1:" (str-cat (lf-bz "LF|T1")
    (str-cat ":ph:" (str-cat (lf-bz "LF|PH")
    (str-cat ":burned:" (str-cat (lf-bz "LF|BUR")
    (str-cat ":now:" (str-cat ts
    (str-cat ":allow:" (lf-allow ts))))))))))))))))))))))))))))

(export "init" lf-init)
(export "reg" lf-reg)
(export "ft_on_transfer" lf-seed)
(export "onleg" lf-onleg)
(export "onref" lf-onref)
(export "wd" lf-wd)
(export "onburn" lf-onburn)
(export "oncx" lf-oncx)
(export "oncy" lf-oncy)
(export "onfin" lf-onfin)
(export "sweep" lf-sweep)
(export "state" lf-state #t)
