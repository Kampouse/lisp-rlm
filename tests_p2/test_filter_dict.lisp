;; Test filter with dict operations

(define (run input)
  (let ((items (list (dict "id" "a" "val" 1) (dict "id" "b" "val" 2) (dict "id" "c" "val" 3))))
    (filter (lambda (i) (= (dict/get i "val") 3)) items)))

;; Expected: list with only {"id": "c", "val": 3}