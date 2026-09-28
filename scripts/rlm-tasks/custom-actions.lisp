;; dream-proposed 1790594994.7506289482
(rlm-register-action "microguide" (lambda (rc)
  (str-concat "\nFor t5_vowels: Use (count-if #'vowel-p string) where vowel-p checks if char is in 'aeiou'.\n"
              "For ta_prime*: First write helper function to check if number is prime, then apply to list.\n"
              "Example prime check: (lambda (n) (loop for i from 2 to (sqrt n) never (= (mod n i) 0)))\n")))
