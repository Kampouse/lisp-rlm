(define (wallet-api-key sub idx) (str-cat "wallet:gw:" sub ":" (to-string idx) ":api_key"))
(define (wallet-acct sub idx) (str-cat "wallet:gw:" sub ":" (to-string idx) ":acct"))

(define (wallet-label sub idx) (str-cat "wallet:gw:" sub ":" (to-string idx) ":label"))

(define (count-wallets sub idx acc)
  (if (> idx 9)
    acc
    (if (nil? (storage-get (wallet-api-key sub idx)))
      acc
      (count-wallets sub (+ idx 1) (+ acc 1)))))

(define (do-register)
  (let* ((_ver (str-cat "" "v2"))
         (sub-raw (json-get-str "google_sub"))
         (sub (str-cat sub-raw ""))
         (existing (storage-get (wallet-api-key sub 0))))
    (if (not (nil? existing))
      (let* ((api-key (storage-get (wallet-api-key sub 0)))
             (acct (storage-get (wallet-acct sub 0))))
        (str-cat "{\"status\":\"ok\",\"api_key\":\"" (if (nil? api-key) "" api-key) "\",\"near_account_id\":\"" (if (nil? acct) "" acct) "\"}"))
      (let* ((resp (outlayer/http-post "https://api.outlayer.fastnear.com/register" "{}"))
             (api-key (str-cat (json-get-str "api_key" resp) ""))
             (near-acct (str-cat (json-get-str "near_account_id" resp) "")))
        (begin
          (storage-set (wallet-api-key sub 0) api-key)
          (storage-set (wallet-acct sub 0) near-acct)
          (str-cat "{\"status\":\"ok\",\"api_key\":\"" api-key "\",\"near_account_id\":\"" near-acct "\"}"))))))

(define (do-balance)
  (let* ((sub-raw (json-get-str "google_sub"))
         (sub (str-cat sub-raw ""))
         (near-acct-raw (storage-get (wallet-acct sub 0))))
    (if (nil? near-acct-raw)
      "{\"status\":\"error\",\"message\":\"account not registered\"}"
      (let* ((near-acct (str-cat near-acct-raw ""))
             (body (str-cat "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"view_account\",\"account_id\":\"" near-acct "\",\"finality\":\"optimistic\"}}"))
             (resp (http-post "https://rpc.mainnet.near.org" body)))
        (if (nil? resp)
          "{\"status\":\"error\",\"message\":\"rpc failed\"}"
          (let* ((result-raw (json-get-str "result" resp))
                 (result (str-cat result-raw ""))
                 (amount-raw (json-get-str "amount" result))
                 (amount (str-cat amount-raw "")))
            (if (nil? amount-raw)
              "{\"status\":\"error\",\"message\":\"account not found on chain\"}"
              (str-cat "{\"status\":\"ok\",\"balance\":\"" amount "\",\"account\":\"" near-acct "\"}"))))))))

(define (do-check)
  (let* ((sub-raw (json-get-str "google_sub"))
         (sub (str-cat sub-raw ""))
         (cnt (count-wallets sub 0 0)))
    (if (= cnt 0)
      "{\"status\":\"ok\",\"exists\":false}"
      (str-cat "{\"status\":\"ok\",\"exists\":true,\"wallet_count\":" (to-string cnt) "}"))))

(define (do-link)
  (let* ((sub-raw (json-get-str "google_sub"))
         (sub (str-cat sub-raw ""))
         (api-key (str-cat (json-get-str "api_key") ""))
         (near-acct (str-cat (json-get-str "near_account_id") "")))
    (if (or (nil? api-key) (nil? near-acct))
      "{\"status\":\"error\",\"message\":\"missing api_key or near_account_id\"}"
      (let* ((cnt (count-wallets sub 0 0))
             (idx cnt))
        (begin
          (storage-set (wallet-api-key sub idx) api-key)
          (storage-set (wallet-acct sub idx) near-acct)
          (str-cat "{\"status\":\"ok\",\"linked\":true,\"wallet_index\":" (to-string idx) ",\"wallet_count\":" (to-string (+ cnt 1)) "}"))))))

(define (clear-wallets sub idx)
  (if (> idx 9)
    0
    (begin
      (storage-set (wallet-api-key sub idx) "")
      (storage-set (wallet-acct sub idx) "")
      (storage-set (wallet-label sub idx) "")
      (clear-wallets sub (+ idx 1)))))

;; unlink wallet by index
(define (do-unlink)
  (let* ((sub-raw (json-get-str "google_sub"))
         (sub (str-cat sub-raw ""))
         (idx-raw (json-get "wallet_index"))
         (idx (if (nil? idx-raw) 0 idx-raw))
         (existing (storage-get (wallet-api-key sub idx))))
    (if (nil? existing)
      "{\"status\":\"error\",\"message\":\"wallet not found\"}"
      (begin
        (storage-set (wallet-api-key sub idx) "")
        (storage-set (wallet-acct sub idx) "")
        (storage-set (wallet-label sub idx) "")
        "{\"status\":\"ok\",\"unlinked\":true}"))))

(define (do-set-label)
  (let* ((sub-raw (json-get-str "google_sub"))
         (sub (str-cat sub-raw ""))
         (idx-raw (json-get "wallet_index"))
         (idx (if (nil? idx-raw) 0 idx-raw))
         (label (str-cat (json-get-str "label") "")))
    (if (nil? label)
      "{\"status\":\"error\",\"message\":\"missing label\"}"
      (begin
        (storage-set (wallet-label sub idx) label)
        "{\"status\":\"ok\"}"))))

(define (build-label-entry sub idx need-comma)
  (let* ((lbl (storage-get (wallet-label sub idx))))
    (if (nil? lbl)
      ""
      (if need-comma
        (str-cat ",{\"index\":" (to-string idx) ",\"label\":\"" lbl "\"}")
        (str-cat "{\"index\":" (to-string idx) ",\"label\":\"" lbl "\"}")))))

(define (scan-labels sub idx need-comma acc)
  (if (> idx 9)
    acc
    (let ((entry (build-label-entry sub idx need-comma)))
      (if (= (str-len entry) 0)
        (scan-labels sub (+ idx 1) need-comma acc)
        (scan-labels sub (+ idx 1) true (str-cat acc entry))))))

(define (do-get-labels)
  (let* ((sub-raw (json-get-str "google_sub"))
         (sub (str-cat sub-raw "")))
    (str-cat "{\"status\":\"ok\",\"labels\":[" (scan-labels sub 0 false "") "]}")))

(define (build-wallet-entry sub idx need-comma)
  (let* ((ak (storage-get (wallet-api-key sub idx)))
         (na (storage-get (wallet-acct sub idx)))
         (lb (storage-get (wallet-label sub idx))))
    (if (nil? ak)
      ""
      (let ((api-key (str-cat ak ""))
            (near-acct (str-cat (if (nil? na) "" na) ""))
            (label (if (nil? lb) "" lb)))
        (if need-comma
          (str-cat ",{\"index\":" (to-string idx) ",\"api_key\":\"" api-key "\",\"near_account_id\":\"" near-acct "\",\"label\":\"" label "\"}")
          (str-cat "{\"index\":" (to-string idx) ",\"api_key\":\"" api-key "\",\"near_account_id\":\"" near-acct "\",\"label\":\"" label "\"}"))))))

(define (scan-wallets sub idx need-comma acc)
  (if (> idx 9)
    acc
    (let ((entry (build-wallet-entry sub idx need-comma)))
      (if (= (str-len entry) 0)
        (scan-wallets sub (+ idx 1) need-comma acc)
        (scan-wallets sub (+ idx 1) true (str-cat acc entry))))))

(define (do-list-wallets)
  (let* ((sub-raw (json-get-str "google_sub"))
         (sub (str-cat sub-raw "")))
    (str-cat "{\"status\":\"ok\",\"wallets\":[" (scan-wallets sub 0 false "") "]}")))

(define (run input)
  (let ((action-num (json-get "action_num")))
    (cond
      ((= action-num 1) (do-register))
      ((= action-num 2) (do-balance))
      ((= action-num 3) (do-check))
      ((= action-num 4) (do-link))
      ((= action-num 5) (do-unlink))
      ((= action-num 6) (do-set-label))
      ((= action-num 7) (do-get-labels))
      ((= action-num 8) (do-list-wallets))
      (true "{\"status\":\"error\",\"message\":\"unknown action\"}"))))


