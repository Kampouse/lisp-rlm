;; wallet_cex.lisp — OutLayer CEX custody wallet
;; Storage: "gw:{sub}" → api_key, "gw:{sub}:acct" → near_account_id
;; Actions: 1=register 2=recover 3=deposit-addr 4=balance 5=transfer

(define (storage-key sub)
  (str-cat "gw:" sub))

(define (acct-key sub)
  (str-cat "gw:" sub ":acct"))

;; --- Register: register or return existing api_key ---

(define (do-register google-sub)
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
                (str-cat "{\"status\":\"ok\",\"api_key\":\"" api-key "\",\"near_account_id\":\"" near-acct "\"}")))))))))

(define (run input)
  (let ((google-sub (json-get-str "google_sub"))
        (action-num (json-get "action_num")))
    (cond
      ((= action-num 1) (do-register google-sub))
      (true "{\"status\":\"error\",\"message\":\"unknown action\"}"))))
