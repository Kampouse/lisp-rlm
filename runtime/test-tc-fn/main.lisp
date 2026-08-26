;; Test: handle-command-like with starts-with?, add-task pattern
(define (my-starts? s prefix)
  ;; Simplified starts-with — just check first char
  (if (= (str-length s) 0) 0
    (if (= prefix "") 1
      (= (str-slice s 0 (str-length prefix)) prefix))))

(define (my-add-task id desc priority)
  (str-concat id ":" desc ":" (to-string priority)))

(define (deep-fn msg)
  (if (= msg "status") "status-ok"
    (if (= msg "tasks") "tasks-list"
      (if (my-starts? msg "add ")
        (my-add-task (str-concat "task-1" msg) (str-slice msg 4 (str-length msg)) 50)
        (if (my-starts? msg "read ")
          (let ((path (str-slice msg 5 (str-length msg))))
            (str-concat "reading: " path))
          (if (my-starts? msg "write ")
            (let ((rest (str-slice msg 6 (str-length msg))))
              (begin
                (storage-set "w" rest)
                (str-concat "wrote: " rest)))
            (begin (storage-set "fallback" msg) (str-concat "unknown: " msg))))))))

(define (run input)
  (str-concat "got: " (deep-fn input)))
