;;; ralph_test.lisp — Test Ralph agent locally (mocked storage)
;;; Run: near-compile test tests/ralph_test.lisp
;;;
;;; Mock storage functions for local testing

;; === Mock Storage (in-memory for tests) ===
(define *storage* (dict))

(define (storage-get key)
  (dict/get *storage* key))

(define (storage-set key value)
  (begin
    (set! *storage* (dict/set *storage* key value))
    "ok"))

(define (outlayer/send-telegram chat-id msg)
  (str-concat "[TG:" chat-id "] " msg))

;; === String helpers ===
(define (str-length s)
  (string-length s))

(define (str-slice s start end)
  (if (< start 0)
    ""
    (if (> end (string-length s))
      (substring s start (string-length s))
      (substring s start end))))

(define (to-string x)
  (if (nil? x)
    "nil"
    (if (number? x)
      (number->string x)
      (if (string? x)
        x
        (if (dict? x)
          (dict-to-json x)
          "unknown")))))

(define (number? x)
  (= (type-of x) "number"))

(define (string? x)
  (= (type-of x) "string"))

(define (dict? x)
  (= (type-of x) "dict"))

(define (nil? x)
  (= x nil))

;; === Test helper ===
(define passed 0)
(define failed 0)

(define (test name actual expected)
  (if (= actual expected)
    (begin
      (set! passed (+ passed 1))
      (str-concat "✓ " name))
    (begin
      (set! failed (+ failed 1))
      (str-concat "✗ " name " — expected: " (to-string expected) ", got: " (to-string actual)))))

(define (run-tests)
  (begin
    ;; Test 1: str-slice
    (println (test "str-slice hello 0 5" (str-slice "hello" 0 5) "hello"))
    (println (test "str-slice hello 1 4" (str-slice "hello" 1 4) "ell"))
    
    ;; Test 2: str-length
    (println (test "str-length hello" (str-length "hello") 5))
    
    ;; Test 3: storage
    (println (test "storage-set" (storage-set "test:key" "value") "ok"))
    (println (test "storage-get" (storage-get "test:key") "value"))
    (println (test "storage-get missing" (storage-get "missing") nil))
    
    ;; Test 4: dict operations
    (println (test "dict/set" (dict/get (dict/set (dict) "a" 1) "a") 1))
    
    ;; Summary
    (println "")
    (println (str-concat "Tests: " (to-string passed) " passed, " (to-string failed) " failed"))
    (if (= failed 0)
      "✅ All tests passed!"
      (str-concat "❌ " (to-string failed) " tests failed"))))

;; Run tests
(run-tests)