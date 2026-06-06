;; Test: Hardcode JSON and get account_id directly
(define (run)
  (let* ((json "{\"account_id\":\"test123\"}")
         (val (json-get-str "account_id" json)))
    val))