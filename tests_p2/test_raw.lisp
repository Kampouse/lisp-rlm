;; Test decode with raw bytes output
(define (run)
  (let* ((bytes "[65,66,67,68,69,70,71]")
         (decoded (json-decode-bytes bytes)))
    decoded))