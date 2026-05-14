(memory 4)

(define (test_const)
  (json-return 42))

(define (test_jsonget)
  (json-return (json-get "amount")))

(export "test_const" test_const true)
(export "test_jsonget" test_jsonget true)
