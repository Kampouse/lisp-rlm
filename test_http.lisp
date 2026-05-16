;; P2/OutLayer: fetch URL from stdin via http-get
(define (run)
  (let ((url (wasi/read_stdin)))
    (let ((response (outlayer/http-get url)))
      (wasi/write_stdout response))))
