;; P2/OutLayer: read URL from stdin, HTTP GET via wasi:http, write response to stdout
(define (run)
  (let ((url (wasi/read_stdin)))
    (let ((response (http/get url)))
      (wasi/write_stdout response))))
