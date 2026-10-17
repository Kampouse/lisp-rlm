(define (p5-trap-v3 who a b d side)
  (begin
    (define acc (loop ((i a) (s "0"))
    (if (> i b)
        s
        (let ((fg (p5-bz (str-cat "6|FG" (str-cat side
                     (str-cat ":" (to-string i))))))
              (fs (p5-bz (p5-fsk side who a b i))))
          (begin
            ;; FS_i ← FG_i NOW: the residual position earns from current
            ;; growth. Delta uses the PRE-reset fs (read-before-write).
            (p5-put (p5-fsk side who a b i) fg)
            (recur (+ i 1)
              (li-add s (p5-mulshr64 (li-sub fg fs) d))))))))
    (p5-put "6|DONE" "1")
    acc))
