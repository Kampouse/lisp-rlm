;; P-256 Passkey Wallet Contract
;; Storage layout:
;;   "pk"    → 33 bytes P-256 compressed SEC1 public key
;;   "nonce" → 4 bytes u32 LE (execution counter)
;;   "sk" + session_pk(33 bytes) → 4 bytes u32 LE (expiry block height)

(define (w_init)
  (near/store-bytes "pk" (near/input))
  (near/store-bytes "nonce" (u32-to-bytes 0)))

(define (w_public_key)
  (near/return_str (hex-encode (near/load-bytes "pk"))))

(define (w_nonce)
  (near/return_str (hex-encode (near/load-bytes "nonce"))))

(define (w_add_session_key)
  (let ((input (near/input)))
    (let ((session_pk (str-slice input 0 33))
          (expiry (bytes-to-u32 (str-slice input 33 37))))
      (near/store-bytes (str-cat "sk" session_pk) (u32-to-bytes expiry)))))

(define (w_execute_session)
  (let ((input (near/input)))
    (let ((session_pk (str-slice input 0 33))
          (nonce (bytes-to-u32 (str-slice input 33 37)))
          (sig_end (str-len input))
          (signed_data (str-slice input 37 (- sig_end 64)))
          (sig (str-slice input (- sig_end 64) sig_end)))
      (let ((stored_nonce (bytes-to-u32 (near/load-bytes "nonce"))))
        (if (!= nonce stored_nonce)
          (near/panic "bad nonce")
          (if (!= (near/p256_verify sig (str-cat (u32-to-bytes nonce) signed_data) session_pk) 1)
            (near/panic "bad sig")
            (begin
              (near/store-bytes "nonce" (u32-to-bytes (+ nonce 1)))
              (near/store-bytes (str-cat "sk" session_pk) ""))))))))

(export "w_init" w_init)
(export "w_public_key" w_public_key)
(export "w_nonce" w_nonce)
(export "w_add_session_key" w_add_session_key)
(export "w_execute_session" w_execute_session)
