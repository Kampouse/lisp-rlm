;; wallet_cex.lisp — OutLayer CEX custody wallet
;; Actions: 1=register, 2=balance, 3=deposit, 4=withdraw

(define (storage-key sub) (str-cat "gw:" sub))
(define (acct-key sub) (str-cat "gw:" sub ":acct"))
(define (bal-key sub) (str-cat "gw:" sub ":bal"))

;; --- Register ---
(define (do-register)
  (let ((google-sub (json-get-str "google_sub")))
    (let ((key (storage-key google-sub)))
      (let ((existing (storage-get key)))
        (if (not (nil? existing))
          (str-cat "{\"status\":\"ok\",\"api_key\":\"" existing "\"}")
          (let ((resp (outlayer/http-post "https://api.outlayer.fastnear.com/register" "{}")))
            (let ((kv (json-extract resp "api_key" "near_account_id")))
              (let ((api-key (vec-nth kv 0))
                    (near-acct (vec-nth kv 1)))
                (begin
                  (storage-set key api-key)
                  (storage-set (acct-key google-sub) near-acct)
                  (str-cat "{\"status\":\"ok\",\"api_key\":\"" api-key "\",\"near_account_id\":\"" near-acct "\"}")
                )
              )
            )
          )
        )
      )
    )
  )
)

;; --- Balance: NEAR RPC query ---
;; NOTE: storage-get nil check crashes in cond dispatch (known emitter bug).
;; Workaround: only call do-balance for registered users (check in dispatch).
(define (do-balance)
  (let ((google-sub (json-get-str "google_sub")))
    (let ((near-acct (storage-get (acct-key google-sub))))
      (let ((body (str-cat "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"view_account\",\"account_id\":\"" near-acct "\",\"finality\":\"optimistic\"}}")))
        (let ((resp (http-post "https://rpc.mainnet.near.org" body)))
          (if (nil? resp)
            "{\"status\":\"error\",\"message\":\"rpc request failed\"}"
            (let ((amount (outlayer/json-get resp "result.amount")))
              (if (nil? amount)
                "{\"status\":\"error\",\"message\":\"failed to parse balance\"}"
                (str-cat "{\"status\":\"ok\",\"balance\":\"" amount "\",\"account\":\"" near-acct "\"}")
              )
            )
          )
        )
      )
    )
  )
)

;; --- Deposit ---
(define (do-deposit)
  (let ((google-sub (json-get-str "google_sub")))
    (let ((bkey (bal-key google-sub)))
      (let ((amount (json-get "amount")))
        (let ((new-bal (storage-increment bkey amount)))
          (str-cat "{\"status\":\"ok\",\"balance\":\"" (to-string new-bal) "\"}")
        )
      )
    )
  )
)

;; --- Withdraw ---
(define (do-withdraw)
  (let ((google-sub (json-get-str "google_sub")))
    (let ((bkey (bal-key google-sub)))
      (let ((current (storage-increment bkey 0)))
        (let ((amount (json-get "amount")))
          (if (< current amount)
            "{\"status\":\"error\",\"message\":\"insufficient balance\"}"
            (let ((new-bal (storage-decrement bkey amount)))
              (let ((destination (json-get-str "destination")))
                (str-cat "{\"status\":\"ok\",\"balance\":\"" (to-string new-bal) "\",\"destination\":\"" destination "\"}")
              )
            )
          )
        )
      )
    )
  )
)

;; --- Dispatch ---
;; Check registration before do-balance to avoid nil check crash
(define (run input)
  (let ((action-num (json-get "action_num")))
    (cond
      ((= action-num 1) (do-register))
      ((= action-num 2)
        (let ((google-sub (json-get-str "google_sub")))
          (let ((existing (storage-get (storage-key google-sub))))
            (if (nil? existing)
              "{\"status\":\"error\",\"message\":\"account not registered\"}"
              (do-balance)))))
      ((= action-num 3) (do-deposit))
      ((= action-num 4) (do-withdraw))
      (true "{\"status\":\"error\",\"message\":\"unknown action\"}")
    )
  )
)
