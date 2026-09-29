;; Nostr identity agent v2 — NON-SPOOFABLE caller binding.
;;
;; Identity is proven by a NEAR-signed execution: the caller attests their
;; challenge by running any OutLayer agent from THEIR OWN account (the exec
;; request is a signed tx; input_data appears in its on-chain logs). The
;; component verifies signer + challenge via RPC before deriving/ signing.
;; Spoofing a caller requires their NEAR key — same bar as taking the account.
;;
;; Ops ({"op":...}):
;;   attest    {"challenge":hex}                     → echo (run this FROM the
;;              caller's own account: outlayer run ... --input '{"op":"attest","challenge":"<ch>"}')
;;   challenge {"caller":"acct"}                     → {"challenge":hex} (fresh, root-bound)
;;   derive    {"caller","tx"}                       → {"pk":...,"root":...}
;;   sign      {"caller","tx","ts","kind","content"} → {"id","sig","pk"}
;;
;; Root: PROTECTED_NOSTR_ROOT (TEE) with demo fallback. sk is NEVER output.
(define (q) (hex-decode "22"))
(define (derive-sk root caller)
  (hex-decode (sha256-hash (str-cat (hex-decode root) (str-cat (hex-decode "1f") caller)))))
(define (pk-of sk) (hex-encode (schnorr-pubkey sk)))
(define (ev-ser pk ts kind content)
  (str-cat "[0," (str-cat (q) (str-cat pk (str-cat (q) (str-cat "," (str-cat ts (str-cat "," (str-cat kind (str-cat ",[]," (str-cat (q) (str-cat content (str-cat (q) "]")))))))))))))
(define (env-or name)
  (let ((v (env/get name)))
    (if (and v (< 63 (str-len (if v v "")))) v "")))
(define (num-str n) (to-string n))
(define (bump-store key)
  (let ((cur (outlayer/storage-get key)))
    (let ((n (if cur (str-to-num cur) 0)))
      (let ((_ (outlayer/storage-set key (num-str (+ n 1)))))
        (num-str n)))))
(define (issue-challenge root caller)
  (let* ((nonce (bump-store "ctr:identity"))
         (seed (str-cat root (str-cat (hex-decode "1f") (str-cat caller (str-cat (hex-decode "1f") nonce)))))
         (ch (sha256-hash seed))
         (_ (outlayer/storage-set (str-cat "ch:" caller) ch)))
    ch))
(define (contains? hay needle)
  ;; str-index-of does a runtime dynamic scan (unlike str-contains,
  ;; which requires a literal needle) — returns byte offset or -1
  (if (< 0 (str-index-of hay needle)) true false))
(define (verify-attestation caller tx)
  (let* ((ch (outlayer/storage-get (str-cat "ch:" caller)))
         (res (if ch
                  (outlayer/raw "tx" (str-cat "[\"" (str-cat tx (str-cat "\",\"" (str-cat caller "\"]")))))
                  "no-challenge"))
         (ok-signer (if ch (contains? res (str-cat "\"signer_id\":\"" (str-cat caller "\""))) false))
         (ok-ch (if ch (contains? res ch) false))
         (ok (if (and ok-signer ok-ch) true false)))
    (if ok (outlayer/storage-delete (str-cat "ch:" caller)) 0)
    ok))
(define (op-derive root caller tx)
  (let* ((ok (verify-attestation caller tx)))
    (if ok
        (let* ((sk (derive-sk root caller))
               (pk (pk-of sk))
               (is-demo (= root "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"))
               (r1 (if is-demo "demo" "real"))
               (o1 (str-cat "{\"pk\":\"" pk))
               (o2 (str-cat o1 (str-cat "\",\"caller\":\"" caller)))
               (o3 (str-cat o2 (str-cat "\",\"root\":\"" r1))))
          (str-cat o3 "\"}"))
        "{\"error\":\"attestation-failed\",\"hint\":\"run attest from the callers own account, then pass that tx hash\"}")))
(define (op-sign root caller tx ts kind content)
  (let* ((ok (verify-attestation caller tx)))
    (if ok
        (let* ((sk (derive-sk root caller))
               (pk (pk-of sk))
               (ser (ev-ser pk ts kind content))
               (idh (sha256-hash ser))
               (aux (hex-decode "0000000000000000000000000000000000000000000000000000000000000000"))
               (sig (schnorr-sign sk (hex-decode idh) aux))
               (sighex (hex-encode sig))
               (o1 (str-cat "{\"id\":\"" idh))
               (o2 (str-cat o1 (str-cat "\",\"sig\":\"" sighex)))
               (o3 (str-cat o2 (str-cat "\",\"pk\":\"" pk)))
               (o4 (str-cat o3 (str-cat "\",\"caller\":\"" caller))))
          (str-cat o4 "\"}"))
        "{\"error\":\"attestation-failed\"}")))
(define (run input)
  (let* ((op (json-get-str "op" input))
         (caller (json-get-str "caller" input))
         (proot (env/get "PROTECTED_NOSTR_ROOT"))
         (nroot (env/get "NOSTR_ROOT"))
         (env-root (if (and proot (< 63 (str-len (if proot proot "")))) proot nroot))
         (root (if (and env-root (< 63 (str-len (if env-root env-root "")))) env-root "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef")))
    (cond
      ((= op "attest") (str-cat "{\"attested\":\"" (str-cat (json-get-str "challenge" input) "\"}")))
      ((= op "challenge") (str-cat "{\"challenge\":\"" (str-cat (issue-challenge root caller) (str-cat "\",\"caller\":\"" (str-cat caller "\"}")))))
      ((= op "sign") (op-sign root caller (json-get-str "tx" input) (json-get-str "ts" input) (json-get-str "kind" input) (json-get-str "content" input)))
      (true (op-derive root caller (json-get-str "tx" input))))))
