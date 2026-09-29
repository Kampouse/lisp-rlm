;; BIP-340 live proof agent (P2 component, self-contained crypto).
;; Input JSON:
;;   {}                       -> official-vector self-test: "MATCH=YES VERIFY=VALID"
;;   {"msg":"<64 hex>"}       -> signs that 32-byte message with the fixed
;;                               demo key (sk=3), returns sig hex + verify.
;;   {"sk":"<64hex>","msg":"<64hex>"} -> signs with the given key.
;; Output: JSON {"sig":..., "verify":1|0} or the self-test string.
(define (run input)
  (let* ((msg-h (json-get-str "msg" input))
         (sk-h (json-get-str "sk" input))
         ;; NB: absent JSON keys yield "" (truthy in this dialect — only
         ;; Bool-false/Nil/Num-0 are falsy), so guard on length, not truthiness
         (msg-h* (if (and msg-h (< 63 (str-len msg-h))) msg-h "0000000000000000000000000000000000000000000000000000000000000000"))
         (sk-h* (if (and sk-h (< 63 (str-len sk-h))) sk-h "0000000000000000000000000000000000000000000000000000000000000003"))
         (sk (hex-decode sk-h*))
         (msg (hex-decode msg-h*))
         (aux (hex-decode "0000000000000000000000000000000000000000000000000000000000000000"))
         (sig (schnorr-sign sk msg aux))
         (sighex (hex-encode sig))
         (ok (schnorr-verify (hex-decode "F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9") sig msg))
         (self-match (if (= sighex "e907831f80848d1069a5371b402410364bdf1c5f8307b0084c55f1ce2dca821525f66a4a85ea8b71e482a74f382d2ce5ebeee8fdb2172f477df4900d310536c0") "YES" "NO"))
         (vfy (if (= ok 1) "VALID" "INVALID"))
         (has-msg (< 0 (str-len (if msg-h msg-h ""))))
         (j1 (if has-msg sighex self-match))
         (j2 (str-cat "{\"" (if has-msg "sig" "match")))
         (j3 (str-cat j2 (str-cat "\":\"" j1)))
         (j4 (str-cat j3 "\",\"verify\":\""))
         (out (str-cat j4 (str-cat vfy "\"}"))))
    out))
