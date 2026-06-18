(define (run input)
  (let ((result (web-search "NEAR Protocol price today")))
    (if (nil? result)
      "search returned nil"
      result)))
