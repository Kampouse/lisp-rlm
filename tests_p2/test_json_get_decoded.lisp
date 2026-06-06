;; Test json-get-str on decoded bytes with nested extraction
(define (run)
  (let* ((bytes "[123, 34, 97, 34, 58, 34, 104, 101, 108, 108, 111, 34, 125]")
         ;; That's {"a":"hello"}
         (decoded (json-decode-bytes bytes)))
    (json-get "a" decoded)))