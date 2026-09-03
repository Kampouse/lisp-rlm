;; e20 — n-ary and/or (2026-09-02): the emitter used to silently drop
;; operands 3+ while the interpreter honored them — the drift that made
;; act="unp" die ERR_ACTION_UNKNOWN in nostr-gov. Also dynamic-needle
;; str-index-of (needle built at runtime via str-cat).
(define (main)
  ;; n-ary and: first falsy wins, last value if all truthy
  (println (and 1 1 1))
  (println (and 1 0 1))
  (println (and 1 1 0))
  (println (and 1 1 2))
  ;; n-ary or: first truthy wins
  (println (or 0 0 1))
  (println (or 0 1 0))
  (println (or 0 2 0))
  ;; 4-ary and mixed comparisons (the nostr-gov action check shape)
  (println (and (!= "unp" "") (!= "unp" "appr") (!= "unp" "unp")))
  (println (and (!= "zzz" "") (!= "zzz" "appr") (!= "zzz" "unp")))
  ;; dynamic needle
  (println (str-index-of "[\"nonce\",\"42\"]" (str-cat "[\"" "nonce" "\",\"")))
  (println (str-index-of "haystack" (str-cat "a" "y")))
  (println (str-index-of "haystack" (str-cat "z" "z")))
  (println (str-index-of "anything" "")))
