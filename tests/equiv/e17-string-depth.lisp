;; e17 — string ops depth: nested str-cat, length, index edge
(define (main)
  (println (str-cat "a" (str-cat "b" (str-cat "c" "d"))))
  (println (str-length (str-cat "ab" "cd")))
  (println (str-length ""))
  (println (= "abc" "abc"))
  (println (= "abc" "abd")))
