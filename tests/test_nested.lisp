(memory 4)

(define (test_flat)
  (json-return (+ (json-get "amount") 1)))

(define (test_nested)
  (json-return (json-get "user.age")))

(define (test_deep)
  (json-return (json-get "a.b.c")))

(export "test_flat" test_flat true)
(export "test_nested" test_nested true)
(export "test_deep" test_deep true)
