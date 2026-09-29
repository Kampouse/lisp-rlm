;; P2 string-shape probe: mirror the chat-agent output assembly exactly.
;; Returns the built JSON so the corruption (missing quotes) is visible.
(define (run input)
  (let* ((idh (json-get-str "id" input))
         (sig (json-get-str "sig" input))
         (n (str-to-num (json-get-str "n" input)))
         (out (str-cat "{\"id\":\"" idh "\",\"sig\":\"" sig
                       "\",\"posted\":" (to-string n) "}")))
    out))
