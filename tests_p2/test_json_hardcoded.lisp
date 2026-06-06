;; Test json-get-str on hardcoded JSON
(define (run)
  (let* ((json "{\"a\":\"hello\"}")
         (val (json-get-str "a" json)))
    (str-cat "val=" val)))