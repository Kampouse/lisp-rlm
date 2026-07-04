;; Test: deeply nested if chain in TC function — like handle-command
(define (deep-nest x)
  ;; 8 levels deep, each with a begin in the else
  (if (= x "a") "result-a"
    (if (= x "b") "result-b"
      (if (= x "c") "result-c"
        (if (= x "d") "result-d"
          (if (= x "e") "result-e"
            (if (= x "f") "result-f"
              (if (= x "g") "result-g"
                (if (= x "h") "result-h"
                  (begin
                    (storage-set "test-key" "nested-ok")
                    "deep-default"))))))))))

(define (run input)
  (str-concat "got: " (deep-nest input)))
