(define (storage-key sub) (str-cat "gw:" sub ""))
(define (acct-key sub) (str-cat "gw:" sub ":acct"))

(define (do-register)
  (let ((sub-raw (json-get-str "google_sub")))
    (let ((sub (str-cat sub-raw "")))
      (let ((key (storage-key sub)))
        (let ((existing (storage-get key)))
          (if (not (nil? existing))
            (str-cat "{\"status\":\"ok\",\"api_key\":\"" existing "\"}")
            (let ((resp (outlayer/http-post "https://api.outlayer.fastnear.com/register" "{}")))
              (let ((api-key (str-cat (json-get-str "api_key" resp) "")))
                (let ((near-acct (str-cat (json-get-str "near_account_id" resp) "")))
                  (begin
                    (storage-set key api-key)
                    (storage-set (acct-key sub) near-acct)
                    (str-cat "{\"status\":\"ok\",\"api_key\":\"" api-key "\",\"near_account_id\":\"" near-acct "\"}")))))))))))

(define (do-balance)
  (let ((sub-raw (json-get-str "google_sub")))
    (let ((sub (str-cat sub-raw "")))
      (let ((near-acct-raw (storage-get (acct-key sub))))
        (if (nil? near-acct-raw)
          "{\"status\":\"error\",\"message\":\"account not registered\"}"
          (let ((near-acct (str-cat near-acct-raw "")))
            (let ((body (str-cat "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"view_account\",\"account_id\":\"" near-acct "\",\"finality\":\"optimistic\"}}")))
              (let ((resp (http-post "https://rpc.mainnet.near.org" body)))
                (if (nil? resp)
                  "{\"status\":\"error\",\"message\":\"rpc failed\"}"
                  (let ((result-raw (json-get-str "result" resp)))
                    (let ((result (str-cat result-raw "")))
                      (let ((amount-raw (json-get-str "amount" result)))
                        (let ((amount (str-cat amount-raw "")))
                          (if (nil? amount-raw)
                            "{\"status\":\"error\",\"message\":\"account not found on chain\"}"
                            (str-cat "{\"status\":\"ok\",\"balance\":\"" amount "\",\"account\":\"" near-acct "\"}")))))))))))))))

(define (run input)
  (let ((action-num (json-get "action_num")))
    (cond
      ((= action-num 1) (do-register))
      ((= action-num 2) (do-balance))
      (true "{\"status\":\"error\",\"message\":\"unknown action\"}"))))
