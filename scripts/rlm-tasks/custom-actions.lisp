;; dream-proposed 1789858185.2433540821
(rlm-register-action "decomp-fold" (lambda (rc)
  (str-concat "\n\n[NEW ACTION] DECOMP-FOLD\n\nThe solver is stuck in infinite recursion on fold-right. DO NOT retry the same fold. Instead:\n1. Extract the accumulator state into a separate variable.\n2. Replace the fold with an explicit loop or recursive helper that carries this state.\n3. Example: (let ([acc (first lst)]) (loop for x in (rest lst) do (setf acc (+ acc x))))\n\nThis breaks the implicit state machine and forces termination.")
  ))
