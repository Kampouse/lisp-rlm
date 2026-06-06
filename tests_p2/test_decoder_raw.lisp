;; Test: Return raw decoded bytes as hex to see memory pattern
(define (run)
  (let* ((bytes "[123, 34, 97, 34, 58, 34, 104, 101, 108, 108, 111, 34, 125]")
         (decoded (json-decode-bytes bytes)))
    ;; Return the decoded string directly
    decoded))