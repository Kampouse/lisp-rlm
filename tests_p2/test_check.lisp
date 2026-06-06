;; Test: Check what's at decode_buf
;; The issue: decode_buf = 131072 might overlap with string literals
;; Let's test if decode_buf is actually being used
(define (run)
  (let* ((bytes "[65]")
         (decoded (json-decode-bytes bytes)))
    (str-cat "Got: " (to-string (str-len decoded)) " bytes")))