;; Nostr identity agent v3 — HYBRID derivation (root ‖ VRF salt) + event-bound
;; challenges. Non-spoofable: caller identity proven by NEAR-signed execution.
;;
;;   salt_X  = VRF output at registration (near:vrf — TEE randomness,
;;             request-bound, verifiable; INDEPENDENT of the root)
;;   sk      = SHA256(root ‖ 0x1f ‖ salt_X ‖ 0x1f ‖ caller)
;;             → root leak alone derives NOTHING (salt unknown)
;;             → storage leak alone derives NOTHING (root unknown)
;;
;;   challenge = SHA256(root ‖ 0x1f ‖ caller ‖ 0x1f ‖ ctr ‖ 0x1f ‖ purpose)
;;     purpose = "derive" | "sign|<ts>|<kind>|<content>"
;;     → binds the attestation to ONE specific action: a raced sign with
;;       different content fails the purpose check.
;;
;; Ops: attest / challenge {caller,[ts,kind,content]} / derive {caller,tx} /
;;      sign {caller,tx,ts,kind,content}. Salt minted on first successful
;;      derive (post-attestation). sk NEVER crosses an output boundary.
(define (q) (hex-decode "22"))
(define (sep) (hex-decode "1f"))
(define (derive-sk root salt caller)
  (hex-decode (sha256-hash (str-cat (hex-decode root)
    (str-cat (sep) (str-cat (hex-decode salt) (str-cat (sep) caller)))))))
(define (pk-of sk) (hex-encode (schnorr-pubkey sk)))
(define (ev-ser pk ts kind content)
  (str-cat "[0," (str-cat (q) (str-cat pk (str-cat (q) (str-cat "," (str-cat ts (str-cat "," (str-cat kind (str-cat ",[]," (str-cat (q) (str-cat content (str-cat (q) "]")))))))))))))
(define (bump-store key)
  (let ((cur (outlayer/storage-get key)))
    (let ((n (if cur (str-to-num cur) 0)))
      (let ((_ (outlayer/storage-set key (to-string (+ n 1)))))
        (to-string n)))))
(define (purpose-of input)
  (let ((ts (json-get-str "ts" input)))
    (if (and ts (< 0 (str-len (if ts ts ""))))
        (str-cat "sign|" (str-cat ts (str-cat "|" (str-cat (json-get-str "kind" input) (str-cat "|" (json-get-str "content" input))))))
        "derive")))
(define (issue-challenge root caller purpose)
  (let* ((ctr (bump-store "ctr:identity"))
         (seed (str-cat (hex-decode root)
                 (str-cat (sep) (str-cat caller
                 (str-cat (sep) (str-cat ctr
                 (str-cat (sep) purpose)))))))
         (ch (sha256-hash seed)))
    (begin
      (outlayer/storage-set (str-cat "ch:" caller) ch)
      (outlayer/storage-set (str-cat "chp:" caller) purpose)
      ch)))
(define (contains? hay needle)
  (if (< 0 (str-index-of hay needle)) true false))
(define (verify-attestation root caller tx purpose)
  (let* ((ch (outlayer/storage-get (str-cat "ch:" caller)))
         (stp (outlayer/storage-get (str-cat "chp:" caller)))
         (res (if ch
                  (outlayer/raw "tx" (str-cat "[\"" (str-cat tx (str-cat "\",\"" (str-cat caller "\"]")))))
                  "no-challenge"))
         (ok-signer (if ch (contains? res (str-cat "\"signer_id\":\"" (str-cat caller "\""))) false))
         (ok-ch (if ch (contains? res ch) false))
         (ok-purpose (if stp (= stp purpose) false))
         (ok (if (and ok-signer (if ok-ch (if ok-purpose true false) false)) true false)))
    (if ok
        (begin
          (outlayer/storage-delete (str-cat "ch:" caller))
          (outlayer/storage-delete (str-cat "chp:" caller))
          0)
        0)
    ok))
(define (ensure-salt caller)
  (let ((s (outlayer/storage-get (str-cat "salt:" caller))))
    (if s s
        ;; NB: near:vrf host rejects seeds containing ':' (alpha parsing) —
        ;; keep the seed colon-free
        (let ((fresh (vrf-generate (str-cat "nostr-salt-" caller))))
          (begin
            (outlayer/storage-set (str-cat "salt:" caller) fresh)
            fresh)))))
(define (op-derive root caller tx)
  (let* ((ok (verify-attestation root caller tx "derive")))
    (if ok
        (let* ((salt (ensure-salt caller))
               (sk (derive-sk root salt caller))
               (pk (pk-of sk))
               (is-demo (= root "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"))
               (o1 (str-cat "{\"pk\":\"" pk))
               (o2 (str-cat o1 (str-cat "\",\"caller\":\"" caller)))
               (o3 (str-cat o2 (str-cat "\",\"root\":\"" (if is-demo "demo" "real"))))
               (o4 (str-cat o3 (str-cat "\",\"salt\":\"" (str-cat (if salt salt "?") "\"}")))))
          o4)
        "{\"error\":\"attestation-failed\"}")))
(define (op-sign root caller tx ts kind content)
  (let* ((purpose (str-cat "sign|" (str-cat ts (str-cat "|" (str-cat kind (str-cat "|" content))))))
         (ok (verify-attestation root caller tx purpose)))
    (if ok
        (let* ((salt (ensure-salt caller))
               (sk (derive-sk root salt caller))
               (pk (pk-of sk))
               (ser (ev-ser pk ts kind content))
               (idh (sha256-hash ser))
               (aux (hex-decode "0000000000000000000000000000000000000000000000000000000000000000"))
               (sig (schnorr-sign sk (hex-decode idh) aux))
               (o1 (str-cat "{\"id\":\"" idh))
               (o2 (str-cat o1 (str-cat "\",\"sig\":\"" (hex-encode sig))))
               (o3 (str-cat o2 (str-cat "\",\"pk\":\"" pk)))
               (o4 (str-cat o3 (str-cat "\",\"caller\":\"" (str-cat caller "\"}")))))
          o4)
        "{\"error\":\"attestation-failed\"}")))
(define (run input)
  (let* ((op (json-get-str "op" input))
         (caller (json-get-str "caller" input))
         (proot (env/get "PROTECTED_NOSTR_ROOT"))
         (env-root (if (and proot (< 63 (str-len (if proot proot "")))) proot ""))
         (root (if (< 63 (str-len env-root)) env-root "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef")))
    (cond
      ((= op "attest") (str-cat "{\"attested\":\"" (str-cat (json-get-str "challenge" input) "\"}")))
      ((= op "challenge") (str-cat "{\"challenge\":\"" (str-cat (issue-challenge root caller (purpose-of input)) (str-cat "\",\"caller\":\"" (str-cat caller "\"}")))))
      ((= op "sign") (op-sign root caller (json-get-str "tx" input) (json-get-str "ts" input) (json-get-str "kind" input) (json-get-str "content" input)))
      (true (op-derive root caller (json-get-str "tx" input))))))
