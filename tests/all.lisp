;; Simple test - no assert macro, just print results
(println (str-concat "1+2 = " (to-string (+ 1 2))))
(println (str-concat "6*7 = " (to-string (* 6 7))))
(println (str-concat "fib10 = " (to-string (fib 10))))
(println (str-concat "square 9 = " (to-string (square 9))))
(println (str-concat "dict get = " (to-string (dict/get (dict "a" 42) "a"))))
(println (str-concat "file /tmp = " (to-string (file/exists? "/tmp"))))
(println "DONE")
