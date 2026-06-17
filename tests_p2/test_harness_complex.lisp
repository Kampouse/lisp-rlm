;; Test harness complexity - just urgency and score

(define (get-default m key default)
  (let ((v (dict/get m key)))
    (if (nil? v) default v)))

(define (urgency intent now)
  (let ((deadline (dict/get intent "deadline"))
        (last (dict/get intent "last-acted")))
    (cond
      ((and deadline (> now deadline)) 10000)
      ((and deadline (> deadline now)) 5000)
      ((and last (< (- now last) 3600000)) 500)
      (else 1000))))

(define (score-intention intent now)
  (let ((u (urgency intent now))
        (e 50))
    (+ (* 70 u) (* 30 e))))

(define (run input)
  (score-intention (dict "id" "test" "deadline" 100000) 50000))

;; Expected: 5000 (deadline > now case)