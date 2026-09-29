;; POST probe: what does the native wasi:http bridge actually send?
;; postman-echo.com/post echoes the method/headers/body it received.
(define (run input)
  (let* ((r1 (http-post "https://postman-echo.com/post" "PING-BODY-1"))
         (r1s (if r1 r1 "?"))
         (r2 (http-post "https://nos.lol" "[\"EVENT\",{\"x\":1}]"))
         (r2s (if r2 r2 "?")))
    (str-cat "{\"echo\":\\\"" (str-slice r1s 0 700)
             "\\\",\\\"nos\\\":\\\"" (str-slice r2s 0 300) "\\\"}")))