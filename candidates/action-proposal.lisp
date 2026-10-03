;; dream-proposed 1790845601.8707480431
;; scoped + de-CL'd 2026-10-01 (cross-task contamination fix — see
;; rlm-scope-action in rlm_runtime.lisp; payload hints are task-family
;; specific so they must not fire on lending tasks).
(rlm-register-action "microguide" (lambda (rc)
  (str-concat "\nFor vowel problems: Use (find (lambda (c) (member c (list \"a\" \"e\" \"i\" \"o\" \"u\"))) (str-split s \"\")) to check characters.\n"
              "For prime problems: Create (define (prime? n) ...) checking divisibility up to (* i i) <= n.\n"
              "Test helper functions individually before combining them.\n")))
(rlm-scope-action "microguide" (list "ta_" "td_prime" "t5_"))
