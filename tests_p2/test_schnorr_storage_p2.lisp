;; P2 storage + crypto: signs with the vector-0 key and stores the sig.
;; Storage round-trips raw binary cleanly (proven Sep 29: NUL-led/trailing
;; bytes survive). Sig is stored as hex TEXT on purpose — readers must
;; decode ONCE (a second hex-encode double-hexes it, the old trap).
;; Output: "stored".
(define (run input)
  (let* ((sk (hex-decode "0000000000000000000000000000000000000000000000000000000000000003"))
         (msg (sha256-hash "gm from p2"))
         (sig (schnorr-sign sk (hex-decode msg) (hex-decode msg)))
         (_ (outlayer/storage-set "last-sig" (hex-encode sig))))
    "stored"))
