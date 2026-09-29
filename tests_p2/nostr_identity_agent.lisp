;; Nostr identity agent — deterministic per-caller keys on OutLayer (P2).
;;
;;   root  : NOSTR_ROOT env var (deploy as PROTECTED hex32 via `outlayer
;;           secrets set --generate NOSTR_ROOT:hex32`). Falls back to a DEMO
;;           root when unset — output says which one was used.
;;   caller: "caller" input field in this version. PRODUCTION NOTE: bind to
;;           the ATTESTED identity (env-signer / request predecessor) instead —
;;           an input field is spoofable by whoever submits the run.
;;
;; Ops (input {"op":...}):
;;   derive: sk = SHA256(root ‖ 0x1f ‖ caller) → returns {"pk":...,"root":"real"|"demo"}
;;           (sk is NEVER returned; sign re-derives it internally)
;;   sign  : builds NIP-01 event [0,pk,ts,kind,[],content], id = SHA256(ser),
;;           sig = BIP-340(sk, id) → returns {"id":...,"sig":...,"pk":...}
(define (q) (hex-decode "22"))
(define (derive-sk root caller)
  (hex-decode (sha256-hash (str-cat (hex-decode root) (str-cat (hex-decode "1f") caller)))))
(define (pk-of sk) (hex-encode (schnorr-pubkey sk)))
(define (ev-ser pk ts kind content)
  (str-cat "[0," (str-cat (q) (str-cat pk (str-cat (q) (str-cat "," (str-cat ts (str-cat "," (str-cat kind (str-cat ",[]," (str-cat (q) (str-cat content (str-cat (q) "]")))))))))))))
(define (ev-id ser) (sha256-hash ser))
(define (op-derive root caller)
  (let* ((sk (derive-sk root caller))
         (pk (pk-of sk))
         (is-demo (= root "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"))
         (r1 (if is-demo "demo" "real"))
         (o1 (str-cat "{\"pk\":\"" pk))
         (o2 (str-cat o1 "\",\"root\":\""))
         (o3 (str-cat o2 r1)))
    (str-cat o3 "\"}")))
(define (op-sign root caller ts kind content)
  (let* ((sk (derive-sk root caller))
         (pk (pk-of sk))
         (ser (ev-ser pk ts kind content))
         (idh (ev-id ser))
         (aux (hex-decode "0000000000000000000000000000000000000000000000000000000000000000"))
         (sig (schnorr-sign sk (hex-decode idh) aux))
         (sighex (hex-encode sig))
         (o1 (str-cat "{\"id\":\"" idh))
         (o2 (str-cat o1 "\",\"sig\":\""))
         (o3 (str-cat o2 sighex))
         (o4 (str-cat o3 "\",\"pk\":\""))
         (o5 (str-cat o4 pk)))
    (str-cat o5 "\"}")))
(define (run input)
  (let* ((op (json-get-str "op" input))
         (caller (json-get-str "caller" input))
         (proot (env/get "PROTECTED_NOSTR_ROOT"))
         (nroot (env/get "NOSTR_ROOT"))
         (env-root (if (and proot (< 63 (str-len proot))) proot nroot))
         (root (if (and env-root (< 63 (str-len env-root))) env-root "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"))
         (is-real (and env-root (< 63 (str-len env-root)))))
    (if (and op (= op "sign"))
        (op-sign root caller (json-get-str "ts" input) (json-get-str "kind" input) (json-get-str "content" input))
        (op-derive root caller))))
