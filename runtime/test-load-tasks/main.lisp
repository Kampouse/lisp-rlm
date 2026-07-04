(define (count-pending tasks)
  (if (nil? tasks) 0
    (let ((task (car tasks)))
      (let ((v (dict/get task "status")))
        (if (nil? v) v v))))))

(define (run input)
  (let ((x (storage-get "ralph:tasks")))
    (let ((elem (json-array-get x 0)))
      (let ((id (json-get "id" elem))
            (status (json-get "status" elem)))
        (let ((t (dict "id" id "status" status)))
          (count-pending (cons t nil)))))))
