;; e02 — strings + str-cat
(define (main)
  (println "hello")
  (println (str-cat "a" "b"))
  (println (str-length "abcd"))
  (println (str-cat "x" (str-cat "y" "z"))))
