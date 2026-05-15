;; Test 4: String operations
(define (main)
  (print (length "hello"))
  (print (substring "hello world" 6 11))
  (print (string-append "foo" "bar")))
;; Expected: 5 world foobar
