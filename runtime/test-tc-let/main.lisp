;; Test: handle-command-like pattern with deep nesting + let + begin + storage
(define (deep-let x)
  (if (= x "a") "result-a"
    (if (= x "b") (begin (storage-set "k1" "v1") "result-b")
      (if (= x "c") 
        (let ((y (str-slice x 0 1)))
          (if (= y "c") 
            (let ((z (str-concat "z:" y)))
              (begin
                (storage-set "k2" z)
                (str-concat "wrote: " z)))
            "no-c"))
        (if (= x "d")
          (let ((p (str-slice x 0 1)))
            (begin
              (storage-set "k3" p)
              "had-colon"))
          (begin (storage-set "k4" "done") "deep-default"))))))

(define (run input)
  (str-concat "got: " (deep-let input)))
