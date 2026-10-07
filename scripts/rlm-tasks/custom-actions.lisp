;; dream-proposed 1791316024.9744050503
(rlm-register-action "microexample" (lambda (rc)
  (str-concat "\nFor REVERSE: Try (reverse '(1 2 3)) → (3 2 1). Use recursion or iteration.\n"
              "For POWER: Implement (pow x n) using multiplication. Handle n=0 separately.\n"
              "For PRIME_SUM: Check divisibility up to sqrt(n). Sum primes in range.\n"
              "For VOWELS: Count 'a','e','i','o','u' (case-insensitive) in string.\n"
              "Use decomposition: break into smaller subproblems.\n")))
