;; Minimal test: just read stdin and return raw bytes count
;; Test if blocking-read works at all

(define (run)
  (let* ((s (stdin-read-line)))
    (str-cat "{\"len\":" (to-string (str-len s)) "}")))