;; P2/OutLayer: read stdin, prepend "echo:", write to stdout
(define (run)
  (let ((input (wasi/read_stdin)))
    (wasi/write_stdout input)))
