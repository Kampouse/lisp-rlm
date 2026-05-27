;; wallet_cex.lisp — OutLayer CEX-like custody wallet
;; Uses worker storage (shared, persistent across all calls)
;;
;; Actions:
;;   {"action_num":1,"google_sub":"12345"}         → register (create wallet)
;;   {"action_num":2,"google_sub":"12345"}         → recover (get existing key)
;;   {"action_num":3,"google_sub":"12345"}         → get deposit address + balance
;;   {"action_num":5,"google_sub":"12345","receiver":"addr","amount":"1000"} → transfer

(define (storage-key sub)
  (str-cat "gw:" sub))

(define (do-register google-sub)
  (let ((key (storage-key google-sub)))
    ;; Check worker storage for existing wallet
    (let ((existing (storage-get-worker key)))
      (if (not (nil? existing))
        ;; Already registered
        (str-cat "{\"status\":\"ok\",\"api_key\":\"" existing "\",\"message\":\"existing\"}")
        ;; Register new via OutLayer API
        (let ((resp (http-post "https://api.outlayer.fastnear.com/register" "{}")))
          (let ((extracted (json-extract resp "api_key" "near_account_id")))
            (let ((api-key (vec-nth extracted 0))
                  (near-acct (vec-nth extracted 1)))
              (if (= (str-len api-key) 0)
                "{\"status\":\"error\",\"message\":\"registration failed\"}"
                (begin
                  ;; Store api_key in worker storage (persists across calls, shared namespace)
                  (storage-set-worker key api-key)
                  (str-cat "{\"status\":\"ok\",\"api_key\":\""
                    api-key "\",\"near_account_id\":\""
                    near-acct "\"}"))))))))))

(define (do-recover google-sub)
  (let ((key (storage-key google-sub)))
    (let ((api-key (storage-get-worker key)))
      (if (nil? api-key)
        "{\"status\":\"not_found\",\"message\":\"no wallet for this google account\"}"
        (str-cat "{\"status\":\"ok\",\"api_key\":\"" api-key "\"}")))))

(define (do-balance google-sub)
  (let ((key (storage-key google-sub)))
    (let ((api-key (storage-get-worker key)))
      (if (nil? api-key)
        "{\"status\":\"error\",\"message\":\"no wallet found\"}"
        ;; Register endpoint returns the account_id for an existing key
        (let ((resp (http-post "https://api.outlayer.fastnear.com/register" "{}")))
          (let ((extracted (json-extract resp "api_key" "near_account_id")))
            (let ((near-acct (vec-nth extracted 1)))
              (if (= (str-len near-acct) 0)
                "{\"status\":\"error\",\"message\":\"could not resolve account\"}"
                ;; Query balance via raw RPC
                (let ((bal-resp (outlayer/raw "query" "{\"request_type\":\"view_account\",\"account_id\":\"PLACEHOLDER\",\"finality\":\"final\"}")))
                  (if (= (str-len bal-resp) 0)
                    "{\"status\":\"error\",\"message\":\"balance query failed\"}"
                    (let ((amount (json-extract bal-resp "amount" "block_hash")))
                      (let ((amt (vec-nth amount 0)))
                        (str-cat "{\"status\":\"ok\",\"amount\":\"" amt
                          "\",\"account_id\":\"" near-acct "\"}")))))))))))))

(define (do-deposit-address google-sub)
  (let ((key (storage-key google-sub)))
    (let ((api-key (storage-get-worker key)))
      (if (nil? api-key)
        "{\"status\":\"error\",\"message\":\"no wallet found\"}"
        ;; Get account_id via register endpoint
        (let ((resp (http-post "https://api.outlayer.fastnear.com/register" "{}")))
          (let ((extracted (json-extract resp "api_key" "near_account_id")))
            (let ((near-acct (vec-nth extracted 1)))
              (str-cat "{\"status\":\"ok\",\"address\":\"" near-acct "\"}"))))))))

(define (do-transfer google-sub receiver amount)
  "{\"status\":\"error\",\"message\":\"transfers coming soon\"}")

;; --- Entry point ---

(define (handle input)
  (let ((google-sub (json-get-str "google_sub"))
        (action-num (json-get "action_num")))
    (if (= (str-len google-sub) 0)
      "{\"status\":\"error\",\"message\":\"missing google_sub\"}"
      (cond
        ((= action-num 1) (do-register google-sub))
        ((= action-num 2) (do-recover google-sub))
        ((= action-num 3) (do-deposit-address google-sub))
        ((= action-num 4) (do-balance google-sub))
        ((= action-num 5)
          (let ((receiver (json-get-str "receiver"))
                (amount (json-get-str "amount")))
            (do-transfer google-sub receiver amount)))
        (true "{\"status\":\"error\",\"message\":\"unknown action\"}")))))
