;; ═══════════════════════════════════════════════════════════════════
;; corpus/erc20.lisp — corpus contract #1: ERC-20-semantics fungible token
;; (NEAR/NEP-141-flavored naming, ERC-20 semantics)
;;
;; STATE MODEL — all state in mock storage as u128 decimal STRINGS:
;;   "supply"            → total supply          (missing = "0")
;;   "bal:<account>"     → account balance        (missing = "0")
;;   "allow:<from>:<to>" → owner→spender allowance (missing = "0")
;;
;; MATH — amounts are strings; ALL arithmetic via u128/{add,sub,lt,eq,
;; is-zero,from-i64,mul} only. NO bare + on amounts. Malformed amount
;; strings are HARD ERRORS (u128 builtins never silently misparse);
;; domain failures (insufficient balance/allowance) use the error model
;; below and leave state UNCHANGED (all checks precede all stores).
;;
;; ERROR MODEL (chosen: empty-string failure) —
;;   Every mutating op returns "" on failure and a NON-EMPTY string on
;;   success (ft-transfer → "1"; ft-mint → new supply; ft-approve → new
;;   allowance; ft-transfer-from → remaining allowance). A non-empty
;;   string is never a failure. (fail? v) = (= (str-length v) 0) — an
;;   EXPLICIT numeric comparison, never truthiness.
;;
;; ZERO-AMOUNT TRANSFERS: no-op success (ERC-20 allows; returns "1",
;;   state untouched by construction — subtracting/adding "0" is exact).
;;
;; LANDMINE COMPLIANCE (GAPS.md):
;;   • numeric 0 is TRUTHY in this dialect — every condition is an
;;     explicit (= x 0) / (u128/lt a b) / (u128/eq a b) / (u128/is-zero s)
;;     / (string? v) comparison. NEVER (if amount ...). Note: no string
;;     content-equality is used anywhere — the wasm path has none.
;;   • T4 closure bug — NO lambdas/closures anywhere in this file; all
;;     mutable state lives in storage, loop accumulators in let only.
;;
;; JSON INPUT WRAPPERS: SKIPPED — the interpreter's mock env does not
;;   support real input capture (near/json_get_str is a stub returning
;;   ""), so the core functions above are the deliverable per contract.
;;
;; str-cat is strings-only on BOTH surfaces (matches wasm_emit: untag
;;   assumes TAG_STR; convert numbers explicitly via to-string /
;;   u128/from-i64 — both exist in interpreter and wasm paths). Composite
;;   keys use NESTED 2-arg str-cat: the type checker registers str-cat as
;;   binary (str→str→str) even though the emitter is variadic — nested
;;   calls type-check AND emit on both surfaces.
;; ═══════════════════════════════════════════════════════════════════

;; ── storage helpers (missing key normalizes to "0") ──────────────
(define (load-str key)
  (let ((v (near/load key)))
    (if (string? v) v "0")))

(define (balance-of acct)      (load-str (str-cat "bal:" acct)))
(define (total-supply)         (load-str "supply"))
(define (allowance-of o s)     (load-str (str-cat "allow:" (str-cat o (str-cat ":" s)))))

(define (set-balance! acct amt)     (near/store (str-cat "bal:" acct) amt))
(define (set-supply! amt)           (near/store "supply" amt))
(define (set-allowance! o s amt)    (near/store (str-cat "allow:" (str-cat o (str-cat ":" s))) amt))

;; ── error model ───────────────────────────────────────────────────
(define (fail? v) (= (str-length v) 0))   ; "" = failure; explicit compare

;; ── core ops ──────────────────────────────────────────────────────
;; (ft-mint to amount) — testnet helper. Returns new total supply.
(define (ft-mint to amount)
  (let ((nb (u128/add (balance-of to) amount))
        (ns (u128/add (total-supply) amount)))
    (set-balance! to nb)
    (set-supply! ns)
    ns))

;; (ft-transfer from to amount) → "1" on success, "" on failure.
;; Failure: bal(from) < amount. State unchanged on failure.
(define (ft-transfer from to amount)
  (let ((b (balance-of from)))
    (if (u128/lt b amount)
        ""
        (begin
          ;; store FROM first, then re-read TO from storage: when from==to
          ;; the re-read sees (b-amount), so the second store restores b —
          ;; a self-transfer is an exact no-op WITHOUT needing string
          ;; equality (wasm path has no content-compare builtin)
          (set-balance! from (u128/sub b amount))
          (set-balance! to (u128/add (balance-of to) amount))
          "1"))))

;; (ft-approve owner spender amount) — OVERWRITE semantics (ERC-20).
;; Returns the new allowance.
(define (ft-approve owner spender amount)
  (set-allowance! owner spender amount)
  amount)

;; (ft-transfer-from spender from to amount) → remaining allowance on
;; success, "" on failure. Failure: allowance < amount OR bal(from) <
;; amount. Allowance decremented by amount; value moves from→to.
(define (ft-transfer-from spender from to amount)
  (let ((a (allowance-of from spender))
        (b (balance-of from)))
    (if (u128/lt a amount)
        ""
        (if (u128/lt b amount)
            ""
            (begin
              ;; same re-read pattern as ft-transfer: self-move restores b
              (set-allowance! from spender (u128/sub a amount))
              (set-balance! from (u128/sub b amount))
              (set-balance! to (u128/add (balance-of to) amount))
              (u128/sub a amount))))))
