(memory 4)

(define (m1) (let ((a 2048) (b 2064)) (begin (fp64/set_int a 2) (fp64/set_int b 3) (fp64/mul a b) (near/return (fp64/get_int a)))))
(export "m1" m1 true)
(define (m1f) (let ((a 2048) (b 2064)) (begin (fp64/set_int a 2) (fp64/set_int b 3) (fp64/mul a b) (near/return (fp64/get_frac a)))))
(export "m1f" m1f true)

(define (d1) (let ((a 2048) (b 2064)) (begin (fp64/set_int a 7) (fp64/set_int b 2) (fp64/div a b) (near/return (fp64/get_int a)))))
(export "d1" d1 true)
(define (d1f) (let ((a 2048) (b 2064)) (begin (fp64/set_int a 7) (fp64/set_int b 2) (fp64/div a b) (near/return (fp64/get_frac a)))))
(export "d1f" d1f true)

(define (s1) (let ((a 2048) (b 2064)) (begin (fp64/set_int a 4) (fp64/sqrt b a) (near/return (fp64/get_int b)))))
(export "s1" s1 true)
(define (s1f) (let ((a 2048) (b 2064)) (begin (fp64/set_int a 4) (fp64/sqrt b a) (near/return (fp64/get_frac b)))))
(export "s1f" s1f true)

(define (s2) (let ((a 2048) (b 2064)) (begin (fp64/set_int a 2) (fp64/sqrt b a) (near/return (fp64/get_int b)))))
(export "s2" s2 true)
(define (s2f) (let ((a 2048) (b 2064)) (begin (fp64/set_int a 2) (fp64/sqrt b a) (near/return (fp64/get_frac b)))))
(export "s2f" s2f true)
