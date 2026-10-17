;; V4: the "obvious" shape — fold loop, then read its accumulator outside
(define (p5-trap-v4 who a b d side)
  (begin
    (loop ((i a) (acc "0"))
      (if (> i b)
          acc
          (recur (+ i 1) acc)))
    (p5-put "6|DONE" "1")
    acc))
