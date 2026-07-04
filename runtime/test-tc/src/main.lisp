;; Test TC with storage operations and complex patterns
(define (build-key prefix idx)
  (str-concat prefix (to-string idx)))

(define (scan-keys idx limit acc)
  (if (= idx limit)
    acc
    (begin
      (storage-set (build-key "item/" idx) "value")
      (scan-keys (+ idx 1) limit (+ acc 1)))))

(define (handle input)
  (scan-keys 0 5 0))
