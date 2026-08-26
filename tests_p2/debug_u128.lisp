;; Debug str-concat with near/load_u128

(define (test_str_concat)
  ;; Test str-concat with simple values
  (near/return_str (str-concat "a=" "1" ", b=" "2")))

(define (test_load_simple)
  ;; Just load and return the pointer
  (let ((loaded-ptr (near/load_u128 "test")))
    (near/return_str (to-string loaded-ptr))))

(define (test_concat_load)
  ;; Concat with loaded value
  (let ((loaded-ptr (near/load_u128 "test")))
    (near/return_str (str-concat "loaded=" (to-string loaded-ptr)))))

(define (test_two_loads)
  ;; Call load_u128 twice and concat both
  (let ((a (near/load_u128 "test")))
    (let ((b (near/load_u128 "test")))
      (near/return_str (str-concat "a=" (to-string a) ", b=" (to-string b))))))

(export "test_str_concat" test_str_concat)
(export "test_load_simple" test_load_simple)
(export "test_concat_load" test_concat_load)
(export "test_two_loads" test_two_loads)