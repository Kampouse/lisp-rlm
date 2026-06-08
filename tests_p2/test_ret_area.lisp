;; Debug: after rpc-view, read raw ret_area bytes and return as hex-ish string
;; ret_area at 131328, 16 bytes: ptr0, len0, ptr1, len1
(define (run)
  ;; Do the rpc-view call
  (rpc-view "system.near" "get_account_info" "{}" "")
  ;; Return just the first arg ptr to verify ret_area
  42)
