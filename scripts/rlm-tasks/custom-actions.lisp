;; dream-proposed 1791205897.1770870686
(rlm-register-action "microdebug" (lambda (rc)
  (str-concat "\nMICRODEBUG: Try these concrete approaches:\n"
              "For runtime errors: Add (print \"Input:\" x) before each operation.\n"
              "For power: Use (dotimes (i exp) (setq result (* result base))).\n"
              "For prime sums: First write (is-prime n) checking divisibility up to sqrt(n).\n"
              "For vowels: Compare each char against '(#\\a #\\e #\\i #\\o #\\u) using member.\n"
              "Test each small function separately before combining them.\n")))
