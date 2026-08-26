;; NOTE: stdin via (json-get-str "key" input) has a bug in the canonical ABI layer.
;; The blocking_read host function writes Result<List<u8>> to RET_AREA,
;; but the pointer field (RET_AREA[4:8]) contains invalid memory address.
;; 
;; WORKAROUND: Use the 2-argument form (json-get-str "key" input) which
;; scans directly from the input buffer without fd_read.
;;
;; Example:
;;   (define (run input)
;;     (let* ((acct (json-get-str "account_id" input)))
;;       (str-cat "{\"received\":\"" acct "\"}")))

(define (run)
  (let* ((acct (json-get-str "account_id" input)))
    (str-cat "{\"received\":\"" acct "\",\"len\":" (to-string (str-len acct)) "}")))
