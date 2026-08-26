;; error: calling a let-shadowed builtin VALUE as a function must be a hard
;; error, not a silent fallback to the builtin underneath.
;; Message: "cannot call 5 as a function (expected a lambda or builtin)",
;; exit 1. The shadow must WIN even in call position — a silent fallback to
;; the real `car` would be the classic wrong-scope bug.
(let ((car 5))
  (println (car (list 1 2))))
