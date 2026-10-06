;; dream-proposed 1791303624.8922929764
(rlm-register-action "microsteps" (lambda (rc)
  (str-concat "\nTry breaking down the problem into micro-steps.\n"
              "For each algorithm:\n"
              "1. Write pseudocode first\n"
              "2. Implement one step at a time\n"
              "3. Test after each step\n"
              "For reverse: build a new list by appending elements in reverse order\n"
              "For power: multiply base 'exponent' times\n"
              "For prime sum: check each number for primality before adding\n"
              "For vowels: check each character against a vowel list\n")))
