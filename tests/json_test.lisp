;; JSON parsing tests — JS-like automatic type detection
;; Run: near-compile test tests/json_test.lisp

;; Test 1: Parse full JSON object
(test "json-parse object"
  (dict/get (json-parse "{\"name\":\"alice\",\"age\":30}") "age")
  30)

(test "json-parse string field"
  (dict/get (json-parse "{\"name\":\"alice\",\"age\":30}") "name")
  "alice")

;; Test 2: Nested access
(test "json-get-in nested"
  (json-get-in "{\"user\":{\"address\":{\"city\":\"mtl\"}}}" "user" "address" "city")
  "mtl")

;; Test 3: json-get single key
(test "json-get int"
  (json-get "{\"x\":42,\"y\":99}" "x")
  42)

(test "json-get string"
  (json-get "{\"x\":42,\"name\":\"bob\"}" "name")
  "bob")

;; Test 4: Missing key → nil
(test "json-get missing key"
  (json-get "{\"x\":1}" "z")
  nil)

;; Test 5: Arrays
(test "json-parse array"
  (first (json-parse "[10,20,30]"))
  10)

;; Test 6: to-json roundtrip
(test "to-json from dict"
  (json-get (to-json (dict "count" 5 "name" "test")) "count")
  5)

;; Test 7: Build JSON from Lisp values
(test "json-build"
  (json-get (json-build (dict "price" 100)) "price")
  100)

;; Test 8: Booleans and null
(test "json-parse bool true"
  (json-parse "true")
  true)

(test "json-parse bool false"
  (json-parse "false")
  false)

(test "json-parse null"
  (json-parse "null")
  nil)
