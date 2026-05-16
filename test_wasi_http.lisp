;; P2/OutLayer: read URL from stdin, HTTP GET, write response to stdout
(define (run)
  (let ((url (wasi/read_stdin)))
    (let ((response (outlayer/http-get url)))
      (wasi/write_stdout response))))
