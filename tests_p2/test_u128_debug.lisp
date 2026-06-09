;; Debug u128/to_str - trace execution
;; Value 1 should produce len=1, ptr=buf+39

(defun test-u128-debug ()
  (let ((addr (u128/from-num 1)))  ; Just 1
    (let ((s (u128/to-str addr)))
      s)))  ; Return the tagged string

(export "test_u128_debug" (func test-u128-debug))