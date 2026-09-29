;; VRF probe: does near:vrf/api generate work on the real OutLayer worker?
(define (run input)
  (let* ((v (vrf-generate "probe-seed"))
         (n (if v (str-len (if v v "")) -1)))
    (str-cat "{\"vrf\":\"" (str-cat (if v v "NIL") (str-cat "\",\"len\":" (str-cat (to-string n) "\"}"))))))
