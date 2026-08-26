;; Minimal test: just read stdin and return raw bytes
;; Test if blocking-read works at all

(define (run)
  (let* ((buf (bytes-alloc 1000)))  ;; Allocate 1000 byte buffer
    (bytes-read stdin buf 1000)      ;; Read up to 1000 bytes from stdin
    (bytes-to-str buf)))             ;; Return as string