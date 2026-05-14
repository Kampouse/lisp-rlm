;; Test JSON in WASM build
(module
  (export "json_parse_test" (json-parse "{\"x\":42}"))
  (export "hello" (lambda () "hi")))
