(define (starts-with? str prefix)
  (let ((plen (str-length prefix)))
    (if (< (str-length str) plen)
      0
      (= (str-slice str 0 plen) prefix))))

(define (handle-cmd msg)
  (if (= msg "status")
    "status-ok"
    (if (= msg "tasks")
      "tasks-ok"
      (if (= msg "reset")
        "reset-ok"
        (if (= msg "run")
          "run-ok"
          (if (= msg "progress")
            "progress-ok"
            (if (starts-with? msg "add task ")
              "add-ok"
              (if (starts-with? msg "read ")
                "read-ok"
                (if (starts-with? msg "list ")
                  "list-ok"
                  (if (starts-with? msg "write ")
                    "write-ok"
                    (str-concat "Unknown: " msg)))))))))))

(define (run input)
  (handle-cmd input))
