;; ═══════════════════════════════════════════════════════════════════
;; corpus/erc20-battery.lisp — executable battery for corpus/erc20.lisp
;; (SCRIPTED PART + 200-op seeded fuzz)
;;
;; SELF-CONTAINED: the interpreter (lisp-run) has no (module ...) include
;; facility, so corpus/erc20.lisp is embedded VERBATIM below (from the
;; "STATE MODEL" header through the end of ft-transfer-from). If you edit
;; the contract, keep this embedded copy byte-identical.
;;
;; Run:  ./target/debug/lisp-run corpus/erc20-battery.lisp
;; All expected u128 constants (expected-output block at the tail) were
;; computed with python3 exact integer arithmetic mirroring the LCG and
;; op dispatch below — NOT by hand.
;;
;; BATTERY STRUCTURE:
;;   Part 1 (scripted, 9 blocks): mint → transfer → overspend fail →
;;     approve+transfer_from → over-allowance fail → over-balance fail →
;;     allowance overwrite → self-transfer + zero-amount → final state.
;;     After EVERY block: conservation assert (sum of balances == supply)
;;     then (println "OK").
;;   Part 2 (fuzz): seed 20260825, 200 ops. Per op the LCG advances 5
;;     times: op = s%4 (0 mint / 1 transfer / 2 approve / 3
;;     transfer_from), from/to/spender = s%4 → alice/bob/carol/dan,
;;     amount = (s%997)·10^21 (u128 string math only). Conservation
;;     asserted after EVERY op. Ends with (println "FUZZ-OK n=200").
;; ═══════════════════════════════════════════════════════════════════

;; ═══════════════════════════════════════════════════════════════════
;; corpus/erc20.lisp — corpus contract #1: ERC-20-semantics fungible token
;; (NEAR/NEP-141-flavored naming, ERC-20 semantics)
;;
;; STATE MODEL — all state in storage as u128 decimal STRINGS via the
;; STRING-SAFE family (near/storage_set/get — bytes-in-bytes-out over raw
;; host fns; values survive fresh-memory transactions). Migrated
;; 2026-08-27 off near/store|load, whose 8-byte tagged-word payload is
;; heap garbage across txs (GAPS erc20 hazard, now closed):
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
;; near/storage_get returns Str("") on miss — the ONLY values ever stored
;; are non-empty decimal strings, so empty ⇔ missing ⇔ "0". Explicit
;; str-length compare (never truthiness — see landmine notes).
(define (load-str key)
  (let ((v (near/storage_get key)))
    (if (= (str-length v) 0) "0" v)))

(define (balance-of acct)      (load-str (str-cat "bal:" acct)))
(define (total-supply)         (load-str "supply"))
(define (allowance-of o s)     (load-str (str-cat "allow:" (str-cat o (str-cat ":" s)))))

(define (set-balance! acct amt)     (near/storage_set (str-cat "bal:" acct) amt))
(define (set-supply! amt)           (near/storage_set "supply" amt))
(define (set-allowance! o s amt)    (near/storage_set (str-cat "allow:" (str-cat o (str-cat ":" s))) amt))

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

;; ═══════════════════════════════════════════════════════════════════
;; BATTERY — Part 1: scripted blocks (all amounts exact decimal u128
;; strings; python3-verified). DEVIATION NOTE: the task's scripted spec
;; was arithmetically infeasible (alice outflows 1.15e24 > 1e24 minted),
;; so block 4 adds one top-up mint (alice +1e24) and block 6 re-approves
;; alice→bob at 1e24 before the over-balance attempt (which also makes
;; block 7's "5e17 replaces 1e24" wording exact). Everything else matches
;; the spec's amounts and expectations.
;; ═══════════════════════════════════════════════════════════════════

(define (sum4)
  (u128/add (u128/add (u128/add (balance-of "alice") (balance-of "bob"))
                      (balance-of "carol"))
            (balance-of "dan")))
;; note: the interpreter has no callable `assert` — conserved? raises via
;; (error ...) instead, which hard-fails the run (equivalent guarantee)
(define (conserved?)
  (if (u128/eq (sum4) (total-supply))
      nil
      (error "CONSERVATION BROKEN")))

;; Block 1 — mint: alice 1e24, bob 2.5e24 → supply 3.5e24
(ft-mint "alice" "1000000000000000000000000")
(ft-mint "bob"   "2500000000000000000000000")
(println (balance-of "alice"))
(println (balance-of "bob"))
(println (total-supply))
(conserved?) (println "OK")

;; Block 2 — transfer alice→bob 0.75e24 → 0.25e24 / 3.25e24
(println (ft-transfer "alice" "bob" "750000000000000000000000"))
(println (balance-of "alice"))
(println (balance-of "bob"))
(conserved?) (println "OK")

;; Block 3 — overspend: bob→alice 99e24 → "" fail, state unchanged
(println (ft-transfer "bob" "alice" "99000000000000000000000000"))
(println (balance-of "alice"))
(println (balance-of "bob"))
(conserved?) (println "OK")

;; Block 4 — top-up mint (deviation, see header), approve alice→bob 1e24,
;;           transfer_from bob alice→carol 0.4e24 → allowance 0.6e24
(ft-mint "alice" "1000000000000000000000000")   ; DEVIATION: top-up
(ft-approve "alice" "bob" "1000000000000000000000000")
(println (ft-transfer-from "bob" "alice" "carol" "400000000000000000000000"))
(println (allowance-of "alice" "bob"))
(println (balance-of "carol"))
(conserved?) (println "OK")

;; Block 5 — transfer_from over-allowance: 0.7e24 > 0.6e24 → "" fail
(println (ft-transfer-from "bob" "alice" "carol" "700000000000000000000000"))
(println (allowance-of "alice" "bob"))
(conserved?) (println "OK")

;; Block 6 — transfer_from over-balance: re-approve 1e24 (deviation),
;;           0.9e24 ≤ allowance but alice has 0.85e24 → "" fail
(ft-approve "alice" "bob" "1000000000000000000000000")
(println (ft-transfer-from "bob" "alice" "dan" "900000000000000000000000"))
(println (allowance-of "alice" "bob"))
(println (balance-of "dan"))
(conserved?) (println "OK")

;; Block 7 — allowance overwrite: approve 5e17 replaces 1e24
(println (ft-approve "alice" "bob" "500000000000000000"))
(println (allowance-of "alice" "bob"))
(conserved?) (println "OK")

;; Block 8 — self-transfer (no-op success) + zero-amount (no-op success)
(println (ft-transfer "alice" "alice" "100000000000000000000000"))
(println (ft-transfer "bob" "alice" "0"))
(println (balance-of "alice"))
(conserved?) (println "OK")

;; Block 9 — final scripted state
(println (balance-of "alice"))
(println (balance-of "bob"))
(println (balance-of "carol"))
(println (balance-of "dan"))
(println (total-supply))
(conserved?) (println "OK")

;; ═══════════════════════════════════════════════════════════════════
;; BATTERY — Part 2: seeded fuzz (200 ops). LCG: s' = (1103515245·s +
;; 12345) mod 2^31, seed 20260825 (all intermediates < 2^61, i64-safe).
;; 5 advances per op → op/from/to/spender/amount selectors. Amounts are
;; (s mod 997)·10^21 u128 strings — always valid. Every op is one of the
;; 4 mutating ops; conservation is asserted after EVERY op.
;; ═══════════════════════════════════════════════════════════════════

;; LCG via u128 string math: 1103515245·s can reach ~2^61 — i64-safe but
;; OVER the 2^60 tagged-payload range guard on bare mul (money-safety,
;; 2026-08-26). Same exact modular sequence, u128 intermediates.
(define (next-seed s)
  (u128/to-i64
   (u128/mod (u128/add (u128/mul (u128/from-i64 s) "1103515245") "12345")
             "2147483648")))
(define (acct i)
  (cond ((= i 0) "alice")
        ((= i 1) "bob")
        ((= i 2) "carol")
        (else "dan")))

(let ((s 20260825) (n 0))
  (while (< n 200)
    (set! s (next-seed s))
    (let ((op (mod s 4)))
      (set! s (next-seed s))
      (let ((from (acct (mod s 4))))
        (set! s (next-seed s))
        (let ((to (acct (mod s 4))))
          (set! s (next-seed s))
          (let ((spender (acct (mod s 4))))
            (set! s (next-seed s))
            (let ((amt (u128/mul (u128/from-i64 (mod s 997))
                                  "1000000000000000000000")))
              (if (= op 0)
                  (ft-mint to amt)
                  (if (= op 1)
                      (ft-transfer from to amt)
                      (if (= op 2)
                          (ft-approve from spender amt)
                          (ft-transfer-from spender from to amt))))
              (conserved?))))))
    (set! n (+ n 1)))
  ;; post-fuzz state (python3-verified constants, see expected block)
  (println (balance-of "alice"))
  (println (balance-of "bob"))
  (println (balance-of "carol"))
  (println (balance-of "dan"))
  (println (total-supply))
  (println "FUZZ-OK n=200"))

;; ── EXPECTED OUTPUT (println of a Str prints with quotes) ─────────
;; block 1
;; "1000000000000000000000000"
;; "2500000000000000000000000"
;; "3500000000000000000000000"
;; "OK"
;; block 2
;; "1"
;; "250000000000000000000000"
;; "3250000000000000000000000"
;; "OK"
;; block 3
;; ""
;; "250000000000000000000000"
;; "3250000000000000000000000"
;; "OK"
;; block 4
;; "600000000000000000000000"
;; "600000000000000000000000"
;; "400000000000000000000000"
;; "OK"
;; block 5
;; ""
;; "600000000000000000000000"
;; "OK"
;; block 6
;; ""
;; "1000000000000000000000000"
;; "0"
;; "OK"
;; block 7
;; "500000000000000000"
;; "500000000000000000"
;; "OK"
;; block 8
;; "1"
;; "1"
;; "850000000000000000000000"
;; "OK"
;; block 9
;; "850000000000000000000000"
;; "3250000000000000000000000"
;; "400000000000000000000000"
;; "0"
;; "4500000000000000000000000"
;; "OK"
;; fuzz final state
;; "850000000000000000000000"
;; "3250000000000000000000000"
;; "4157000000000000000000000"
;; "22699000000000000000000000"
;; "30956000000000000000000000"
;; "FUZZ-OK n=200"
;; ""FUZZ-OK n=200""                 ← driver echo: lisp-run prints the
;;                                     last top-level form's value; the
;;                                     final let evaluates to this string
;;                                     (println returns its argument)
