(define (run)
  ;; Debug: return first 8 bytes of memory at STDIN_BUF (32768) as hex
  ;; and the length at STDIN_LEN (98304)
  (let* ((stdin-len (byte-at 98304))
         (stdin-len2 (byte-at 98305))
         (stdin-len3 (byte-at 98306))
         (stdin-len4 (byte-at 98307))
         (b0 (byte-at 32768))
         (b1 (byte-at 32769))
         (b2 (byte-at 32770))
         (b3 (byte-at 32771)))
    (str-cat "{\"len_at_98304\":\"" 
      (to-hex stdin-len) (to-hex stdin-len2) (to-hex stdin-len3) (to-hex stdin-len4)
      "\",\"stdin\":\""
      (to-hex b0) (to-hex b1) (to-hex b2) (to-hex b3)
      "\"}")))