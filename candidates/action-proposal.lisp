;; dream-proposed 1791308416.3054089546
(rlm-register-action "microdebug" (lambda (rc)
  (str-concat "\nMICRO-DEBUG: Examine intermediate results and edge cases.\n"
              "For 'ok' tasks with low Q: Check if your solution is optimal or meets all requirements.\n"
              "For runtime errors: Add print statements at each step to isolate the failure point.\n"
              "Test with minimal inputs first, then gradually increase complexity.\n")))
