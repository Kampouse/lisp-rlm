;; Test: Check if single digit works
(define (run)
  (let* ((bytes "[65]")
         (decoded (json-decode-bytes bytes)))
    decoded))