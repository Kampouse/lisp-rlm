;; P2/OutLayer: read input, call rpc/status, write result to stdout
(define (run)
  (wasi/read_stdin)  ;; consume stdin (unused)
  (let ((result (rpc/status)))
    (wasi/write_stdout result)))
