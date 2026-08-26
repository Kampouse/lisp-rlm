;; Test find-best function from harness

(define (get-default m key default)
  (let ((v (dict/get m key)))
    (if (nil? v) default v)))

(define (score-intention intent now)
  (dict/set intent "score" 100))

(define (find-best intentions now)
  (if (nil? intentions) nil
    (if (nil? (cdr intentions))
      (score-intention (car intentions) now)
      (let ((head (score-intention (car intentions) now))
            (tail-best (find-best (cdr intentions) now)))
        (let ((hs (dict/get head "score"))
              (ts (dict/get tail-best "score")))
          (if (> (if (nil? hs) 0 hs) (if (nil? ts) 0 ts))
            head
            tail-best))))))

(define (run input)
  (find-best (list (dict "id" "a") (dict "id" "b")) 1000))