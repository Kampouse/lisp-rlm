;; Google OAuth — WASM P2 version (inlayer)
;; ========================================
;; Full OAuth refresh token flow as a P2 WASM component
;; Uses outlayer/ builtins: http-post, json-get, str-concat
;;
;; Compile: cargo run --release --bin emit_p2 -- examples/google_oauth_wasm.lisp
;; Run:     inlayer run /tmp/emitted_p2.wasm
;;
;; Replace YOUR_* values before running

(define (main)
  ;; Build form body: grant_type=refresh_token&refresh_token=REFRESH&client_id=ID&client_secret=SECRET
  (let ((body (outlayer/str-concat
    (outlayer/str-concat
      "grant_type=refresh_token&refresh_token=YOUR_REFRESH_TOKEN&client_id=YOUR_CLIENT_ID&client_secret=YOUR_CLIENT_SECRET"
      "")
    "")))

    ;; POST to Google token endpoint
    (let ((resp (outlayer/http-post
      "https://oauth2.googleapis.com/token"
      body
      "application/x-www-form-urlencoded")))

      ;; Extract access_token from JSON response
      (let ((token (outlayer/json-get resp "access_token")))

        ;; Use token to call Google userinfo API
        (let ((auth-header (outlayer/str-concat "Bearer " token)))
          (let ((user-info (outlayer/http-get
            (outlayer/str-concat "https://www.googleapis.com/oauth2/v1/userinfo?access_token=" token))))

            ;; Print the user info
            (wasi/write_stdout user-info)))))))
