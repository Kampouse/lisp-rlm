(define (run)
  (let* (
    (args "{\"account_id\":\"alice.near\"}")
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (len (str-len account-raw))
    )
    ; Return raw memory at ret_area to see what's there
    (print "len: ")
    (print len)
    (print "\n")
    ; Return the length so we can see if it's inline
    len))
