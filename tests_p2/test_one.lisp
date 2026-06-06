;; Test: Single byte to check decoder output
(define (run)
  (let* ((bytes "[72]")
         (decoded (json-decode-bytes bytes)))
    decoded))