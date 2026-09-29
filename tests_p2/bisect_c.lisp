;; bisect C: LITERAL-url http-post — the native wasi:http POST bridge path
;; (sentinel 104 hoists the URL; no outlayer host import for POST).
(define (run input)
  (let* ((res (http-post "https://nos.lol" "[\"EVENT\",{}]")))
    (str-cat "{\"res\":\"" (if res (str-slice res 0 64) "?") "\"}")))
