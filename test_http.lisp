;; P2/OutLayer: fetch weather from stdin URL via http-get
(define (run)
  (let ((url (wasi/read_stdin)))
    (let ((response (http/get url)))
      (wasi/write_stdout response))))
