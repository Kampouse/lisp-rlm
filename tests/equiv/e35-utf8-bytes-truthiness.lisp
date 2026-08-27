;; e35 — UTF-8 byte semantics + truthiness parity (round 5, 2026-08-27
;; rulings: bytes-everywhere; Num(0) falsy — both surfaces already agreed
;; on the latter, this pins it).
(define (main)
  ;; byte lengths (both surfaces: UTF-8 bytes, not chars)
  (println (str-length "héllo"))
  (println (str-length "日本語"))
  (println (str-length ""))
  ;; byte-indexed substring, clamped, lossy mid-codepoint
  (println (str-substring "héllo" 0 2))
  (println (str-substring "héllo" 0 6))
  (println (str-substring "日本語" 3 6))
  (println (str-substring "abc" 2 1))
  (println (str-substring "abc" 5 9))
  ;; ASCII-only case mapping — non-ASCII passes through
  (println (str-upcase "héllo"))
  (println (str-downcase "HÉLLO"))
  (println (str-upcase "abc123"))
  ;; ASCII-whitespace-only trim
  (println (str-trim "  x\t"))
  (println (str-trim "\u{00a0}x")) ;; NBSP is NOT trimmed (byte semantics)
  ;; byte-offset index-of
  (println (str-index-of "héllo" "l"))
  (println (str-index-of "héllo" "ll"))
  (println (str-index-of "héllo" "z"))
  ;; byte-parity ops on UTF-8 input
  (println (str-contains "日本語" "本"))
  (println (str-starts-with "héllo" "hé"))
  (println (str-ends-with "héllo" "llo"))
  (println (str-replace "aéa" "a" "b"))
  (println (str-split "aé:b" ":"))
  ;; truthiness parity (0 falsy, everything else truthy)
  (println (if 0 "t" "f"))
  (println (if 1 "t" "f"))
  (println (if "" "t" "f"))
  (println (if "0" "t" "f"))
  (println (not 0))
  (println (and 0 2))
  (println (or 0 3)))
