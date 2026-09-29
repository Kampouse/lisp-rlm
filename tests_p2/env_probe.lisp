;; env probe v2 — the 2026-09-29 identity probe guessed WRONG env names
;; (SENDER_ID, SIGNER_ID…) and missed NEAR_SENDER_ID / NEAR_PREDECESSOR_ID,
;; which scripts/nostr_seal proved host-injected from the request_execution
;; tx on 2026-09-22 (commit f86c310: spoof of alice.testnet by kampy.testnet
;; rejected on-chain). This probe dumps both + every plausible neighbour so
;; the chat agent can bind callers with ZERO challenge round-trips.
(define (env-or name)
  (let ((v (env/get name)))
    (if (and v (< 0 (str-len (if v v "")))) v "?")))
(define (run input)
  (let* ((c1 (env-or "NEAR_SENDER_ID"))
         (c2 (env-or "NEAR_PREDECESSOR_ID"))
         (sp (json-get-str "caller" input))
         (o1 (str-cat "{\"NEAR_SENDER_ID\":\"" c1))
         (o2 (str-cat o1 (str-cat "\",\"NEAR_PREDECESSOR_ID\":\"" c2)))
         (o3 (str-cat o2 (str-cat "\",\"input_caller\":\"" (if sp sp "?"))))
         (o4 (str-cat o3 (str-cat "\",\"root\":\"" (if (< 0 (str-len (env-or "PROTECTED_NOSTR_ROOT"))) "set" "unset")))))
    (str-cat o4 "\"}")))
