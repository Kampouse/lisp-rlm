;; Test json-get-str with 2-arg version (explicit buffer)
;; This bypasses the cabi_realloc issue

(define (run)
  (let* ((buf (bytes-alloc 1000))  ;; Allocate buffer for JSON
         (acct (json-get-str "account_id" buf)))  ;; Use explicit buffer
    (str-cat "{\"account\":\"" acct "\"}")))