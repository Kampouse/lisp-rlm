;; wallet_auth.lisp — OutLayer handler for Google-authenticated custody wallets
;;
;; stdin: {"action":"register","google_sub":"12345"}
;; stdin: {"action":"recover","google_sub":"12345"}
;;
;; _start wrapper calls last defined function with stdin as argument.

;; --- JSON helpers ---

(define (json-read-until-rec json pos delim cur)
  (if (>= cur (str-len json))
    (str-slice json pos cur)
    (if (= (str-index-of (str-slice json cur (+ cur 1)) delim) 0)
      (str-slice json pos cur)
      (json-read-until-rec json pos delim (+ cur 1)))))

(define (json-read-until json pos delim)
  (json-read-until-rec json pos delim pos))

;; Build search pattern: "key":" 
(define (make-key-search key)
  (str-concat "\"" (str-concat key "\":\"")))

(define (extract-json-str json key)
  (let ((search (make-key-search key)))
    (let ((idx (str-index-of json search)))
      (if (< idx 0)
        ""
        (json-read-until json (+ idx (str-len search)) "\"")))))

(define (json-action? json action)
  (let ((pattern (str-concat "\"action\":\"" (str-concat action "\""))))
    (>= (str-index-of json pattern) 0)))

;; --- Predicates ---

(define (str-empty? s)
  (= (str-len s) 0))

(define (nil? val)
  (= val 4))

;; --- Storage ---

(define (make-storage-key sub)
  (str-concat "wallet:" sub))

;; --- Actions ---

(define (do-register google-sub)
  (let ((existing (storage-get (make-storage-key google-sub))))
    (if (not (nil? existing))
      (str-concat "{\"status\":\"ok\",\"api_key\":\"" (str-concat existing "\",\"message\":\"existing\"}"))
      (let ((resp (http-post "https://api.outlayer.fastnear.com/register" "{}")))
        (let ((api-key (extract-json-str resp "api_key")))
          (let ((near-acct (extract-json-str resp "near_account_id")))
            (if (str-empty? api-key)
              "{\"status\":\"error\",\"message\":\"registration failed\"}"
              (begin
                (storage-set (make-storage-key google-sub) api-key)
                (str-concat "{\"status\":\"ok\",\"api_key\":\""
                  (str-concat api-key (str-concat "\",\"near_account_id\":\""
                    (str-concat near-acct "\"}"))))))))))))

(define (do-recover google-sub)
  (let ((key (storage-get (make-storage-key google-sub))))
    (if (nil? key)
      "{\"status\":\"not_found\",\"message\":\"no wallet for this google account\"}"
      (str-concat "{\"status\":\"ok\",\"api_key\":\"" (str-concat key "\"}")))))

;; --- Entry point (last define = called by _start with stdin) ---

(define (handle input)
  (let ((google-sub (extract-json-str input "google_sub")))
    (if (str-empty? google-sub)
      "{\"status\":\"error\",\"message\":\"missing google_sub\"}"
      (cond
        ((json-action? input "register") (do-register google-sub))
        ((json-action? input "recover")  (do-recover google-sub))
        (true "{\"status\":\"error\",\"message\":\"unknown action\"}")))))
