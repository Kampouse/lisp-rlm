;; bisect B: A + a DYNAMIC-url http-post (import 21) — the suspect surface
(define (run input)
  (let* ((sender (env/get "NEAR_SENDER_ID"))
         (snd (if sender sender "?"))
         (relay (json-get-str "relay" input))
         (url (if relay relay "https://nos.lol"))
         (res (http-post url "[\"EVENT\",{}]")))
    (str-cat "{\"snd\":\"" snd "\",\"res\":\"" (if res (str-slice res 0 48) "?") "\"}")))
