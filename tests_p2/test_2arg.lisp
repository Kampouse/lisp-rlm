(define (run)
  ;; Test 2-arg json-get-str with explicit buffer
  (let* ((buf (str-alloc 256))  ;; allocate buffer at RUNTIME_HEAP_PTR
         (acct (json-get-str "account_id" buf)))
    acct))