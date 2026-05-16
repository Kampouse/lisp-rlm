;; Weather checker for OutLayer
;; Input (stdin): {"city":"Montreal"}
;; Output (stdout): raw weather string from wttr.in

(define (run)
  (http-get "https://wttr.in/Montreal?format=%t"))
