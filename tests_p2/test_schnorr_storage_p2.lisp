;; P2 storage + crypto: seals the signing key, signs, stores sig (combined core path).
(define (run input)
  (let* ((sk0 (outlayer/storage-get "sk"))
         (sk (if sk0 sk0 (hex-decode "0000000000000000000000000000000000000000000000000000000000000003")))
         (msg (sha256-hash "gm from p2"))
         (sig (schnorr-sign sk (hex-decode msg) (hex-decode msg)))
         (_ (outlayer/storage-set "last-sig" (hex-encode sig))))
    "stored"))
