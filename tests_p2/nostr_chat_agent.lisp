;; Nostr chat agent v4 — NEAR_SENDER_ID-bound caller + pk33 cache + relay
;; publish. The v3 challenge/attest dance is GONE: on the request_execution
;; path the host injects NEAR_SENDER_ID from the tx itself (input cannot
;; forge it — proven on-chain 2026-09-22, commit f86c310, and re-proven for
;; lisp p2 components via tests_p2/env_probe.lisp on 2026-09-29). One exec
;; per message, non-spoofable by construction.
;;
;; Identity (unchanged from v3, proven):
;;   salt_X = VRF output at registration (TEE randomness, stored)
;;   sk     = SHA256(root ‖ 0x1f ‖ salt_X ‖ 0x1f ‖ sender)
;;   pk33   = SEC1(0x02/0x03 ‖ x) cached at pk33:<sender> — the parity bit
;;            feeds schnorr-sign-pk so sign skips its internal P=d·G mult
;;            (3 EC mults → 1 per message; ~57M → ~20M instructions).
;;
;; Ops (caller ALWAYS from env; input caller fields are ignored):
;;   pk {}                              -> {"pk":..,"sender":..}
;;   chat {content,ts,nonce,room,r}     -> signs NIP-01 kind-1 event with
;;        tags [["t",room],["nonce",n]], POSTs ["EVENT",ev] to the relay
;;        (r: 0=nos.lol default, 1=relay.damus.io — LITERAL URLs: the
;;        outlayer host's dynamic http-post import (21) fails component
;;        instantiation on the worker, so relays are compile-time baked
;;        and ride the native wasi:http POST bridge), returns
;;        {"id","sig","pk","posted"}
;;
;; Fail-closed posture: with a real PROTECTED_NOSTR_ROOT secret set, a
;; missing NEAR_SENDER_ID (HTTPS/CLI paths that carry no tx identity)
;; is REJECTED. The demo-root fallback accepts input caller — local dev
;; and self-tests only.
(define (sep) (hex-decode "1f"))
(define (zeros32) (hex-decode "0000000000000000000000000000000000000000000000000000000000000000"))
(define (derive-sk root salt sender)
  (hex-decode (sha256-hash (str-cat (hex-decode root)
    (str-cat (sep) (str-cat (hex-decode salt) (str-cat (sep) sender)))))))
(define (ensure-salt sender)
  (let ((s (outlayer/storage-get (str-cat "salt:" sender))))
    (if s s
        ;; near:vrf host rejects seeds containing ':' — keep colon-free
        (let ((fresh (vrf-generate (str-cat "nostr-salt-" sender))))
          (begin
            (outlayer/storage-set (str-cat "salt:" sender) fresh)
            fresh)))))
(define (ensure-pk33 sender sk)
  (let ((c (outlayer/storage-get (str-cat "pk33:" sender))))
    (if (if c (= 66 (str-len c)) false)
        (hex-decode c)
        (let ((p33 (schnorr-pubkey33 sk)))
          (begin
            (outlayer/storage-set (str-cat "pk33:" sender) (hex-encode p33))
            p33)))))
(define (x-only pk33) (str-slice pk33 1 33))
(define (tags-ser room nonce)
  ;; INNER list only — ev-ser/ev-json add the enclosing [ ] (NIP-01: the
  ;; tags field is one array of tag-arrays).
  (str-cat "[\"t\",\"" room "\"],[\"nonce\",\"" nonce "\"]"))
(define (ev-ser pk ts kind tags content)
  ;; content arrives PRE-QUOTED (json-quote output includes the quotes)
  (str-cat "[0,\"" pk "\"," ts "," kind ",[" tags "]," content "]"))
(define (ev-json id pk ts kind tags content sig)
  ;; content arrives PRE-QUOTED (json-quote output includes the quotes)
  (str-cat "{\"id\":\"" id
            "\",\"pubkey\":\"" pk
            "\",\"created_at\":" ts
            ",\"kind\":" kind
            ",\"tags\":[" tags
            "],\"content\":" content
            ",\"sig\":\"" sig
            "\"}"))
(define (err m) (str-cat "{\"error\":\"" (str-cat m "\"}")))
(define (post-relay which body)
  ;; Literal URLs ONLY (see header): dynamic-URL posts ride outlayer host
  ;; import 21, which the worker refuses to instantiate. Two baked relays,
  ;; selected by the input flag r.
  (if (= which "1")
      (http-post "https://relay.damus.io" body)
      (http-post "https://nos.lol" body)))
(define (valid-nonce n) (and (< 0 (str-len n)) (< (str-len n) 33)))
(define (valid-room r) (and (< 0 (str-len r)) (< (str-len r) 65)))
(define (resolve-caller input)
  ;; Identity precedence:
  ;;  1. env == a trusted router -> tx came through a router contract; honor
  ;;     its forwarded caller. Trust is NOT baked in: the deployer configures
  ;;     TRUSTED_ROUTERS (comma-separated, via OutLayer secrets) — any number
  ;;     of independent routers/frontends can serve users, and swapping one
  ;;     is a secrets update, not a wasm rebuild.
  ;;  2. env set (direct tx)    -> env wins, input caller ignored (spoof-proof:
  ;;     only a mid-chain router can compose caller; a direct tx's env is the
  ;;     tx signer, so mallory calling directly stays mallory).
  ;;  3. no env (HTTPS/CLI)     -> prod: reject; demo-root: dev fallback.
  (let* ((envs (env/get "NEAR_SENDER_ID"))
         (proot (env/get "PROTECTED_NOSTR_ROOT"))
         (snd (if envs envs ""))
         (prod (< 63 (str-len (if proot proot ""))))
         (inp (json-get-str "caller" input))
         (routers (env/get "TRUSTED_ROUTERS"))
         (rl (if routers routers ""))
         (via-router (str-contains (str-cat "," rl ",")
                                   (str-cat "," snd ","))))
    (if (and (< 0 (str-len snd)) via-router)
        ;; fail-closed: a trusted-router request MUST carry the forwarded
        ;; caller. Silent fallback to snd would sign as the router itself
        ;; (alice/carol footgun 2026-09-30) — return empty so run() rejects.
        (if (and inp (< 0 (str-len inp))) inp "")
        (if (< 0 (str-len snd)) snd
            (if prod "" (if inp inp ""))))))
(define (root-of)
  (let* ((proot (env/get "PROTECTED_NOSTR_ROOT"))
         (env-root (if (and proot (< 63 (str-len (if proot proot "")))) proot "")))
    (if (< 63 (str-len env-root)) env-root "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef")))
(define (op-pk sender)
  (let* ((salt (ensure-salt sender))
         (sk (derive-sk (root-of) salt sender))
         (pk33 (ensure-pk33 sender sk))
         (pk (hex-encode (x-only pk33))))
    (str-cat "{\"pk\":\"" pk "\",\"sender\":\"" sender "\"}")))
(define (op-chat sender content ts nonce room rchoice)
  (if (if (valid-nonce nonce) (if (valid-room room) (< (str-len content) 800) false) false)
      (let* ((salt (ensure-salt sender))
             (sk (derive-sk (root-of) salt sender))
             (pk33 (ensure-pk33 sender sk))
             (pk (hex-encode (x-only pk33)))
             (tags (tags-ser room nonce))
             (qc (json-quote content))
             (ser (ev-ser pk ts "1" tags qc))
             (idh (sha256-hash ser))
             ;; aux = SHA256(room ‖ 0x1f ‖ nonce): deterministic per
             ;; (room,nonce) — retries of the SAME message reuse the same
             ;; nonce → same sig → relays dedupe; a new nonce = fresh aux.
             (aux (hex-decode (sha256-hash (str-cat room (str-cat (sep) nonce)))))
             (sig (hex-encode (schnorr-sign-pk sk pk33 (hex-decode idh) aux)))
             (res (post-relay rchoice (str-cat "[\"EVENT\"," (ev-json idh pk ts "1" tags qc sig) "]")))
             (ok (if res (if (< 0 (str-index-of res "\"OK\"")) 1 0) 0)))
        (str-cat "{\"id\":\"" idh "\",\"sig\":\"" sig
          "\",\"pk\":\"" pk "\",\"posted\":" (to-string ok) "}"))
      (err "invalid-chat-args")))
(define (op-profile sender meta ts nonce)
  ;; kind-0 metadata event (NIP-01: latest wins per pubkey). content = raw
  ;; metadata JSON text, no tags. Same deterministic-aux trick as chat:
  ;; same nonce → same sig → relay dedupe on retries; fresh nonce = update.
  (if (if (valid-nonce nonce) (< (str-len meta) 600) false)
      (let* ((salt (ensure-salt sender))
             (sk (derive-sk (root-of) salt sender))
             (pk33 (ensure-pk33 sender sk))
             (pk (hex-encode (x-only pk33)))
             (qc (json-quote meta))
             (ser (ev-ser pk ts "0" "" qc))
             (idh (sha256-hash ser))
             (aux (hex-decode (sha256-hash (str-cat "profile" (str-cat (sep) nonce)))))
             (sig (hex-encode (schnorr-sign-pk sk pk33 (hex-decode idh) aux))))
        (str-cat "{\"id\":\"" idh "\",\"sig\":\"" sig
          "\",\"pk\":\"" pk "\",\"kind\":0}"))
      (err "invalid-profile-args")))
(define (run input)
  (let* ((op (json-get-str "op" input))
         (sender (resolve-caller input)))
    (if (< 0 (str-len sender))
        (cond
          ((= op "chat") (op-chat sender (json-get-str "content" input) (json-get-str "ts" input)
                                 (json-get-str "nonce" input) (json-get-str "room" input) (json-get-str "r" input)))
          ((= op "profile") (op-profile sender (json-get-str "meta" input) (json-get-str "ts" input)
                                        (json-get-str "nonce" input)))
          ((= op "ser") (let* ((salt (ensure-salt sender))
                                (sk (derive-sk (root-of) salt sender))
                                (pk (hex-encode (x-only (ensure-pk33 sender sk))))
                                (tags (tags-ser (json-get-str "room" input) (json-get-str "nonce" input))))
                          (ev-ser pk (json-get-str "ts" input) "1" tags (json-quote (json-get-str "content" input)))))
          ((= op "version") (str-cat "{\"v\":\"5\"}"))
          ((= op "pk") (op-pk sender))
          (true (op-pk sender)))
        (err "no-identity-path"))))
