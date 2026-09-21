;; dream-proposed 1790025160.9627690315
(rlm-register-action "microstep" (lambda (rc)
  (str-concat "\nTry breaking the problem into micro-steps:\n"
    "For t5_vowels: Create a vowel list first, then check each character\n"
    "For prime_sum: Write a helper to check if a number is prime, then sum\n"
    "For primeprod: Same prime check, but multiply instead of sum\n"
    "For reverse: Use recursion or built-in reverse function\n"
    "Start with the simplest case, then build up complexity.\n")))
