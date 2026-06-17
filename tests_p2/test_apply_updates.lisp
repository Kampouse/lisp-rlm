;; Test apply-updates function

(define (apply-updates intent updates)
  (cond
    ((nil? updates) intent)
    (else (apply-updates
         (dict/set intent (car (car updates)) (car (cdr (car updates))))
         (cdr updates)))))

(define (run input)
  (let ((intent (dict "id" "test" "count" 0)))
    (apply-updates intent (list (list "count" 1) (list "status" "done")))))

;; Expected: {"id": "test", "count": 1, "status": "done"}