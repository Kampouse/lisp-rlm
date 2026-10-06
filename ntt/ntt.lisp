;;;
;;; ntt.lisp — Wormhole NTT ported to NEAR in lisp-rlm (MVP, one contract)
;;; 2026-10-05. burn-and-mint native supply + VAA guardian quorum verify
;;; (ecrecover->keccak addr) + replay protection + rate-limited inbound
;;; with queue and release window.
;;;
;;; Storage: G:i guardian hex | GS:N GS:TH | EM:chain | b:acct u128 dec
;;; V:chain:seq replay | IN/OUT:CAP/USED/START | Q:n queue | SEQ counter
;;; Payload: 01 | amount u256BE | rlen | recipient ascii
;;;
(define WIN 60)

(define (sget k d) (default (near/storage_get k) d))
(define (sput k v) (near/storage_set k v))
(define (srem k) (near/storage_remove k))
(define (fail m) (near/log m) (near/abort m))

;; BE int decode
(define (be32-f s o i)
  (if (= i 4) 0
      (+ (* (be32-f s o (+ i 1)) 256) (byte-at s (+ o i)))))
(define (be32 s o) (be32-f s o 0))
(define (be16 s o) (+ (* (byte-at s o) 256) (byte-at s (+ o 1))))
(define (be64-f s o i)
  (if (= i 8) 0
      (+ (* (be64-f s o (+ i 1)) 256) (byte-at s (+ o i)))))
(define (be64h s o) (be64-f s o 0))

;; u256-BE at [o,o+32) -> u128 decimal string, high half must be zero
(define (u256->u128 s o)
  (if (not (= (be64h s o) 0)) (fail "ERR_OVERFLOW_HI")
      (if (not (= (be64h s (+ o 8)) 0)) (fail "ERR_OVERFLOW_HI")
          (u256-low s (+ o 16)))))
(define (u256-low s p)
  ;; FLAT unrolled BE fold (b0 most significant)
  (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (u128/add (u128/mul (itoa (byte-at s (+ p 0))) "256") (itoa (byte-at s (+ p 1)))) "256") (itoa (byte-at s (+ p 2)))) "256") (itoa (byte-at s (+ p 3)))) "256") (itoa (byte-at s (+ p 4)))) "256") (itoa (byte-at s (+ p 5)))) "256") (itoa (byte-at s (+ p 6)))) "256") (itoa (byte-at s (+ p 7)))) "256") (itoa (byte-at s (+ p 8)))) "256") (itoa (byte-at s (+ p 9)))) "256") (itoa (byte-at s (+ p 10)))) "256") (itoa (byte-at s (+ p 11)))) "256") (itoa (byte-at s (+ p 12)))) "256") (itoa (byte-at s (+ p 13)))) "256") (itoa (byte-at s (+ p 14)))) "256") (itoa (byte-at s (+ p 15)))))

;; u128 decimal -> 32-byte BE binary (16 zero + 16 bytes)
(define (hx) "0123456789abcdef")
(define (byte-hex b)
  (str-cat (str-slice (hx) (/ b 16) (+ (/ b 16) 1))
           (str-slice (hx) (mod b 16) (+ (mod b 16) 1))))
(define (rev-s s i)
  (if (< i 0) ""
      (str-cat (str-slice s i (+ i 1)) (rev-s s (- i 1)))))
(define (enc-lo i x)
  (if (= i 16) ""
      (str-cat (hex-decode (byte-hex (str->num (u128/mod x "256"))))
               (enc-lo (+ i 1) (u128/div x "256")))))
(define (enc-u256 v)
  (str-cat (str-repeat (hex-decode "00") 16) (rev-s (enc-lo 0 v) 15)))

;; ---- init: g0,g1,g2 guardian hex; threshold; em1 emitter for chain 1 ----
(define (init)
  (let ((th (default (near/json_get_str "threshold") "2"))
        (g0 (default (near/json_get_str "g0") ""))
        (g1 (default (near/json_get_str "g1") ""))
        (g2 (default (near/json_get_str "g2") ""))
        (em1 (default (near/json_get_str "em1") ""))
        (ic (default (near/json_get_str "in_cap") "1000")))
    (sput "GS:TH" th)
    (sput "G:0" (str-downcase g0))
    (if (> (str-len g1) 0) (sput "G:1" (str-downcase g1)) 0)
    (if (> (str-len g2) 0) (sput "G:2" (str-downcase g2)) 0)
    (sput "GS:N" (if (> (str-len g2) 0) "3" (if (> (str-len g1) 0) "2" "1")))
    (if (> (str-len em1) 0) (sput "EM:1" (str-downcase em1)) 0)
    (sput "IN:CAP" ic) (sput "IN:USED" "0") (sput "IN:START" (itoa (str->num (near/block_timestamp))))
    (sput "SEQ" "0") (sput "Q:HEAD" "0") (sput "Q:TAIL" "0")
    (near/return "INIT_OK")))

;; ---- token side ----
(define (bal-of a) (sget (str-cat "b:" a) "0"))
(define (mint-to)
  (let ((acct (default (near/json_get_str "account_id") ""))
        (amt (default (near/json_get_str "amount") "0")))
    (sput (str-cat "b:" acct) (u128/add (bal-of acct) amt))
    (near/return (bal-of acct))))
(define (ft-balance-of)
  (near/return (bal-of (default (near/json_get_str "account_id") ""))))

;; ---- outbound: burn + store observation, return payload hex ----
(define (transfer-out)
  (let ((amt (default (near/json_get_str "amount") "0"))
        (recip (default (near/json_get_str "recipient") ""))
        (from (near/predecessor_account_id)))
    (if (u128/lt (bal-of from) amt) (fail "ERR_INSUFFICIENT")
        (let ((seq (u128/add (sget "SEQ" "0") "1")))
          (sput (str-cat "b:" from) (u128/sub (bal-of from) amt))
          (sput "SEQ" seq)
          (let ((payload (str-cat (hex-decode "01") (enc-u256 amt)
                                  (hex-decode (byte-hex (str-len recip))) recip)))
            (sput (str-cat "OUT:" seq) (hex-encode payload))
            (near/return (str-cat "SEQ=" seq " PAYLOAD=" (hex-encode payload))))))))

;; ---- guardian quorum ----
(define (recover-addr digest sig v)
  (let ((pk (near/ecrecover_pk digest sig v 0)))
    (if (= (str-len pk) 0) ""
        (str-downcase (hex-encode
                        (str-slice (near/keccak256 (str-slice pk 1 65)) 12 32))))))
(define (count-quorum vaa digest n i)
  (if (>= i n) 0
      (let ((so (+ 6 (* 65 i))))
        (let ((v (byte-at vaa (+ so 64))))
          (let ((addr (recover-addr digest (str-slice vaa so (+ so 64)) v)))
            (let ((want (sget (str-cat "G:" (itoa i)) "")))
              (+ (if (= addr want) 1 0)
                 (count-quorum vaa digest n (+ i 1)))))))))

;; ---- inbound rate limit + mint or queue ----
(define (apply-inbound payload)
  (let ((pid (byte-at payload 0)))
    (if (not (= pid 1)) (fail "ERR_PAYLOAD_ID")
        (let ((amt (u256->u128 payload 1))
              (rlen (byte-at payload 33)))
          (let ((recip (str-slice payload 34 (+ 34 rlen))))
            (let ((now (str->num (near/block_timestamp))))
              (let ((start (str->num (sget "IN:START" "0"))))
                (if (> (- now start) WIN)
                    (begin (sput "IN:START" (itoa now)) (sput "IN:USED" "0"))
                    0)
                (let ((used (sget "IN:USED" "0")) (cap (sget "IN:CAP" "1000")))
                  (if (u128/gt (u128/add used amt) cap)
                      (let ((tail (sget "Q:TAIL" "0")))
                        (sput (str-cat "Q:" tail) (hex-encode payload))
                        (sput "Q:TAIL" (u128/add tail "1"))
                        "QUEUED")
                      (begin
                        (sput "IN:USED" (u128/add used amt))
                        (sput (str-cat "b:" recip) (u128/add (bal-of recip) amt))
                        "MINTED"))))))))))

;; ---- receive_vaa ----
(define (receive-vaa)
  (let ((vaah (default (near/json_get_str "vaa") "")))
    (if (= (str-len vaah) 0) (fail "ERR_NO_VAA")
        (let ((vaa (hex-decode vaah)))
          (if (not (= (byte-at vaa 0) 1)) (fail "ERR_VERSION")
              (let ((n (byte-at vaa 5)))
                (let ((bo (+ 6 (* 65 n))))
                  (let ((digest (near/keccak256 (str-slice vaa bo (str-len vaa)))))
                    (let ((q (count-quorum vaa digest n 0)))
                      (near/log (str-cat "quorum " (itoa q) " of " (sget "GS:TH" "0")))
                      (if (< q (str->num (sget "GS:TH" "0"))) (fail "ERR_QUORUM")
                          (let ((em-chain (be16 vaa (+ bo 8))))
                            (let ((em-addr (str-downcase
                                              (hex-encode (str-slice vaa (+ bo 10) (+ bo 42))))))
                              (let ((want (sget (str-cat "EM:" (itoa em-chain)) "")))
                                (if (= want "") (fail "ERR_CHAIN")
                                    (if (not (= want em-addr)) (fail "ERR_EMITTER")
                                        (let ((seq (be64h vaa (+ bo 42))))
                                          (let ((rk (str-cat "V:" (itoa em-chain) ":" (itoa seq))))
                                            (if (not (= (sget rk "") "")) (fail "ERR_REPLAY")
                                                (begin
                                                  (sput rk "1")
                                                  (near/return (apply-inbound
                                                                 (str-slice vaa (+ bo 51) (str-len vaa)))))))))))))))))))))))

;; ---- queue release ----
(define (release-inbound)
  (let ((head (sget "Q:HEAD" "0")) (tail (sget "Q:TAIL" "0")))
    (if (not (u128/lt head tail)) (near/return "QUEUE_EMPTY")
        (let ((now (str->num (near/block_timestamp))))
          (let ((start (str->num (sget "IN:START" "0"))))
            (if (<= (- now start) WIN) (near/return "WINDOW_NOT_PASSED")
                (let ((ph (sget (str-cat "Q:" head) "")))
                  (begin
                    (srem (str-cat "Q:" head))
                    (sput "Q:HEAD" (u128/add head "1"))
                    (near/return (apply-inbound (hex-decode ph)))))))))))

(define (get-status)
  (near/return
    (str-cat "seq " (sget "SEQ" "0")
             " in " (sget "IN:USED" "0") "/" (sget "IN:CAP" "1000")
             " q " (sget "Q:HEAD" "0") "-" (sget "Q:TAIL" "0")
             " th " (sget "GS:TH" "0") " n " (sget "GS:N" "0"))))

(export "init" init)
(export "mint_to" mint-to)
(export "ft_balance_of" ft-balance-of)
(export "transfer_out" transfer-out)
(export "receive_vaa" receive-vaa)
(export "release_inbound" release-inbound)
(export "get_status" get-status #t)
