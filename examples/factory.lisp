;; Wallet Factory Contract (traditional pattern)
;; Deploys wallet code per-subaccount from factory storage.
;;
;; Flow:
;;   1. Call f_init with wallet WASM as input (once)
;;   2. Call f_create with args: suffix_len(4B u32) + suffix_bytes + pk_bytes(32B)
;;      - Creates subaccount: {suffix}.{factory_account}
;;      - Deploys wallet WASM from storage
;;      - Calls w_init with public key
;;      - Forwards attached deposit to subaccount

(define (f_init)
  (begin
    (near/store-bytes "code" (near/input))
    (near/return_str "ok")))

(define (f_create)
  (let ((input (near/input)))
    (let ((slen (bytes-to-u32 (str-slice input 0 4))))
      (let ((suffix (str-slice input 4 (+ 4 slen)))
            (pk (str-slice input (+ 4 slen) (+ 36 slen))))
        (let ((subacct (str-cat (str-cat suffix ".") (near/current_account_id))))
          (let ((_dep (near/attached_deposit)))
            (let ((p (near/batch subacct)))
              (near/batch-create-account p)
              (near/batch-deploy p (near/load-bytes "code"))
              (near/batch-call p "w_init" pk 64 300000000000000)
              (near/return subacct))))))))

(export "f_init" f_init)
(export "f_create" f_create)
