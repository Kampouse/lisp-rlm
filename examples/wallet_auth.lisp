;; wallet_auth.lisp — OutLayer handler for Google-authenticated custody wallets
;;
;; stdin JSON parsed by runtime scanner; action via numeric flag
;; {"action_num":1,"google_sub":"12345"}  → register
;; {"action_num":2,"google_sub":"12345"}  → recover
;;
;; Constraints: no str=, no str-contains, no str-index-of in emit.
;; json-get-str uses runtime scanner (key literal only).

;; --- Storage key ---

(define (storage-key sub)
  (str-concat "wallet:" sub))

;; --- Actions ---

(define (do-register google-sub)
  (let ((key (storage-key google-sub)))
    (let ((existing (storage-get key)))
      ;; nil = tagged 4
      (if (not (nil? existing))
        ;; Already registered — return existing key
        (str-concat "{\"status\":\"ok\",\"api_key\":\"" (str-concat existing "\",\"message\":\"existing\"}"))
        ;; Register new via OutLayer API
        (let ((resp (http-post "https://api.outlayer.fastnear.com/register" "{}")))
          (let ((extracted (json-extract resp "api_key" "near_account_id")))
            (let ((api-key (vec-nth extracted 0))
                  (near-acct (vec-nth extracted 1)))
              ;; api-key is "" on miss (str-len = 0)
              (if (= (str-len api-key) 0)
                "{\"status\":\"error\",\"message\":\"registration failed\"}"
                (begin
                  (storage-set key api-key)
                  (str-concat "{\"status\":\"ok\",\"api_key\":\""
                    (str-concat api-key (str-concat "\",\"near_account_id\":\""
                      (str-concat near-acct "\"}")))))))))))))

(define (do-recover google-sub)
  (let ((key (storage-key google-sub)))
    (let ((api-key (storage-get key)))
      (if (nil? api-key)
        "{\"status\":\"not_found\",\"message\":\"no wallet for this google account\"}"
        (str-concat "{\"status\":\"ok\",\"api_key\":\"" (str-concat api-key "\"}"))))))

;; --- Entry point (last define = called by _start with stdin) ---

(define (handle input)
  (let ((google-sub (json-get-str "google_sub"))
        (action-num (json-get "action_num")))
    (if (= (str-len google-sub) 0)
      "{\"status\":\"error\",\"message\":\"missing google_sub\"}"
      (cond
        ((= action-num 1) (do-register google-sub))
        ((= action-num 2) (do-recover google-sub))
        (true "{\"status\":\"error\",\"message\":\"unknown action\"}")))))
