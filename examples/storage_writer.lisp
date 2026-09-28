;; storage_writer.lisp — test fixture: writes one k/v pair per call
(define (m-put)
  (let ((input (near/input)))
    (begin
      (near/storage_write (json-get-str "k" input) (json-get-str "v" input))
      (near/json_return_str "ok"))))

(export "put" m-put false)
