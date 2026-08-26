;; T16 — same-name shadow chains 6 and 8 levels deep, with set! at the
;; innermost level and restoration checks after each scope pops
;;
;; Ground rules: (2) constants hand-computed level by level (python3 below);
;; (4) every shadow level must isolate — inner writes never leak outward,
;; and popping a scope restores the previous binding exactly.

;; 1. depth-8 accumulating chain, set! only at the deepest level
;;    levels: 1 → 11 → 22 → 122 → 100 → 25 → 100 → (set! → 344), pop → 1
(define base 1000)
(let ((x 1))
  (println x)
  (let ((x (+ x 10)))
    (println x)
    (let ((x (* x 2)))
      (println x)
      (let ((x (+ x 100)))
        (println x)
        (let ((x (- x 22)))
          (println x)
          (let ((x (/ x 4)))
            (println x)
            (let ((x (+ x 75)))
              (println x)
              (let ((x (* x 3)))
                (set! x (+ x 44))
                (println x))))))))       ; 344 — closes lets 8..2
  (println x))                           ; 1 — depth-1 binding restored
(println base)                           ; 1000 — untouched throughout

;; 2. depth-6 chain mixing a function PARAM with let-shadowing
;;    p=7: v: 8 → 24 → 17 → 117 → 234; result 234 + 7 = 241
(define (f p)
  (let ((v (+ p 1)))
    (let ((v (* v 3)))
      (let ((v (- v p)))
        (let ((v (+ v 100)))
          (let ((v (* v 2)))
            (+ v p)))))))
(println (f 7))                            ; 241
(println (f 0))                            ; v: 1 → 3 → 3 → 103 → 206; 206+0

;; 3. depth-6 REBINDING chain (each level rebinds from its own literal, not
;;    accumulating) — innermost adds 1: all outer levels must still see 2
(let ((x 2))
  (let ((x 2))
    (let ((x 2))
      (let ((x 2))
        (let ((x 2))
          (let ((x 2))
            (println (+ x 1)))))))
  (println x))                             ; 2 — outer levels untouched
