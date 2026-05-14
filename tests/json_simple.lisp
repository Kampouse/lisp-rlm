;; Simple JSON test — no embedded quotes
(test "json-parse simple int"
  (json-get "{\"x\":42}" "x")
  42)
