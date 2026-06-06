;; Test json-get-str with decoded buffer address
(define (run)
  (let* ((bytes "[123, 34, 97, 34, 58, 34, 104, 101, 108, 108, 111, 34, 125]")
         (decoded (json-decode-bytes bytes))
         ;; json-get-str takes (key json-buffer)
         (val (json-get-str "a" decoded)))
    (str-cat "JSON len: " (to-string (str-len decoded)) " | val=" val)))