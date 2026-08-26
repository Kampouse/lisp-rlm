;; Test env-var on P2 — no http-get so we go through finish_outlayer → build_p2_with_adapter path
(define (run)
  (env-var "NEAR_SENDER_ID"))
