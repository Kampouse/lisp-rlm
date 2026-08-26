(define (get_credits)
  (let ((inp (near/input)))
    (let ((acct-len (bytes-to-u32 (str-slice inp 0 4))))
      (near/return_str (to-string acct-len)))))
