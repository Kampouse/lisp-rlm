;; Test json-get-str on small JSON
;; bytes = [123,34,97,34,58,49,125] = a-object
(define (run)
  (let* ((bytes "[123, 34, 97, 34, 58, 49, 125]")
         (decoded (json-decode-bytes bytes))
         (val (json-get-str "a" decoded)))
    val))