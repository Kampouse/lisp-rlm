;; Test env/signer and env/predecessor in outlayer-p2
(define (run signer pred)
  (str-cat (str-cat "{\"signer\":\"" signer) "\"}"))
