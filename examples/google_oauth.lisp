;; Google OAuth via refresh token exchange
;; ========================================
;; Flow: refresh_token → access_token → Google API call
;;
;; Run with: ./target/release/rlm examples/google_oauth.lisp
;; Or compile to WASM P2: cargo run --release --bin emit_p2 -- examples/google_oauth.lisp

;; --- Configuration (replace with your values) ---
(define client-id "YOUR_CLIENT_ID.apps.googleusercontent.com")
(define client-secret "YOUR_CLIENT_SECRET")
(define refresh-token "YOUR_REFRESH_TOKEN")

;; --- Step 1: Build form-encoded body ---
;; Google token endpoint requires application/x-www-form-urlencoded, NOT JSON
(define token-body
  (str-concat "grant_type=refresh_token&refresh_token="
    (str-concat refresh-token
      (str-concat "&client_id="
        (str-concat client-id
          (str-concat "&client_secret=" client-secret))))))

;; --- Step 2: Exchange refresh token for access token ---
;; 3rd arg = content-type override (defaults to application/json, but Google needs form-encoded)
(define token-response
  (http-post "https://oauth2.googleapis.com/token" token-body "application/x-www-form-urlencoded"))

;; --- Step 3: Parse access token ---
(define token-data (json-parse token-response))
(define access-token (dict/get token-data "access_token"))

;; --- Step 4: Use access token — fetch user profile ---
(define profile
  (http-get
    (str-concat "https://www.googleapis.com/oauth2/v2/userinfo?access_token="
      access-token)))

;; --- Output ---
profile
