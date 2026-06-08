;; Test ed25519-sign with hex secret key (32 bytes)
(define (run)
  (ed25519-sign "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08" "hello"))
