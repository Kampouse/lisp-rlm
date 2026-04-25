;;; rlm.scm — Fractal RLM in pure Guile Scheme
;;; Usage: GLM_API_KEY=... guile rlm.scm "your task here"

(use-modules (rnrs bytevectors)
             (ice-9 textual-ports)
             (ice-9 string-fun)
             (ice-9 popen)
             (srfi srfi-1))

;; Config
(define API-KEY (or (getenv "GLM_API_KEY") (error "Set GLM_API_KEY")))
(define API-URL "https://api.z.ai/api/coding/paas/v4/chat/completions")
(define MODEL (or (getenv "RLM_MODEL") "glm-5.1"))
(define MAX-DEPTH (string->number (or (getenv "RLM_MAX_DEPTH") "4")))
(define MAX-RETRIES (string->number (or (getenv "RLM_MAX_RETRIES") "3")))
(define rlm-tokens 0)
(define rlm-calls 0)

;; JSON escape
(define (js-esc s)
  (let* ((s1 (string-replace-substring s "\\" "\\\\"))
         (s2 (string-replace-substring s1 "\"" "\\\""))
         (s3 (string-replace-substring s2 "\n" "\\n")))
    (string-append "\"" s3 "\"")))

;; Extract content from JSON response (manual parsing)
(define (extract-content json)
  (let ((pos (string-contains json "\"content\":\"")))
    (if (not pos)
        #f
        (let loop ((i (+ pos 11)) (out '()))
          (if (>= i (string-length json))
              (list->string (reverse out))
              (let ((c (string-ref json i)))
                (if (and (eqv? c #\\) (< (+ i 1) (string-length json)))
                    (let ((next (string-ref json (+ i 1))))
                      (loop (+ i 2)
                            (cons (if (eqv? next #\n) #\newline next) out)))
                    (if (eqv? c #\")
                        (list->string (reverse out))
                        (loop (+ i 1) (cons c out))))))))))

;; LLM call via curl
(define (llm prompt . opts)
  (let* ((sys (if (null? opts) "You are a helpful assistant." (car opts)))
         (body (string-append
                "{\"model\":\"" MODEL "\","
                "\"messages\":["
                "{\"role\":\"system\",\"content\":" (js-esc sys) "},"
                "{\"role\":\"user\",\"content\":" (js-esc prompt) "}"
                "],\"max_tokens\":8192}")))
    (call-with-output-file "/tmp/rlm_req.json"
      (lambda (p) (display body p)))
    (system (string-append
             "curl -s -X POST " API-URL
             " -H 'Content-Type: application/json'"
             " -H 'Authorization: Bearer " API-KEY "'"
             " -d @/tmp/rlm_req.json"
             " -o /tmp/rlm_resp.json 2>/dev/null"))
    (set! rlm-calls (+ rlm-calls 1))
    (let* ((resp (call-with-input-file "/tmp/rlm_resp.json" get-string-all))
           (content (extract-content resp)))
      (set! rlm-tokens (+ rlm-tokens (quotient (string-length resp) 4)))
      (or content ""))))

;; Safe eval
(define (safe-eval str)
  (catch #t
    (lambda ()
      (let ((port (open-input-string str)))
        (let loop ((results '()))
          (let ((form (read port)))
            (if (eof-object? form)
                (cons 'ok (reverse results))
                (loop (cons (primitive-eval form) results)))))))
    (lambda (key . args)
      (cons 'error (format #f "~a: ~a" key args)))))

;; Clean markdown fences
(define (clean code)
  (let ((s (string-trim-both code)))
    (if (string-prefix? "```" s)
        (let* ((nl (string-index s #\newline))
               (after (if nl (substring s (+ nl 1)) ""))
               (cleaned (if (string-suffix? "```" after)
                           (substring after 0 (- (string-length after) 3))
                           after)))
          (string-trim-both cleaned))
        s)))

(define (has-final? code)
  (string-contains code "(final "))

;; System prompt
(define SYS
  "You are an autonomous agent writing Guile Scheme code.
Rules:
- Return ONLY Scheme code. No English, no markdown, no explanations.
- Use (final value) to return your final answer.
- Use (assert expr) to verify your work — without assert, answer is UNVERIFIED.
- Use (llm \"prompt\") for one-shot LLM calls.
- Use (rlm \"task\") to spawn a sub-RLM for complex subtasks.
Available: + - * / modulo, string-append, string-length, substring,
  string-contains, string-split, map, filter, length, append, reverse,
  list, cons, car, cdr, assoc, for-each,
  (system \"cmd\") for shell,
  (call-with-input-file path get-string-all) to read files,
  (call-with-output-file path (lambda (p) (display content p))) to write files.")

;; Try to solve in one shot
(define (try-solve task)
  (let ((code (llm (string-append "Task: " task
                     "\nReturn ONLY Scheme code ending with (final value).")
                   SYS)))
    (display (format #f "  Generated: ~a\n"
              (substring code 0 (min 80 (string-length code)))))
    (force-output)
    (if (not (has-final? code))
        (cons 'error "no final")
        (safe-eval (clean code)))))

;; Decompose failed task into 2 subtasks
(define (decompose task)
  (let ((resp (llm (string-append
      "Split into exactly 2 subtasks. Output ONLY two lines:\nTASK: <sub1>\nTASK: <sub2>\n"
      "Task: " task)
     "Output only TASK: lines.")))
    (let ((lines (filter (lambda (l) (string-contains l "TASK:"))
                         (string-split resp #\newline))))
      (if (>= (length lines) 2)
          (let ((strip (lambda (l)
                         (let ((p (string-contains l "TASK:")))
                           (string-trim (substring l (+ p 5)))))))
            (list (strip (list-ref lines 0))
                  (strip (list-ref lines 1))))
          (list task task)))))

;; Synthesize child results
(define (synthesize task results)
  (llm (string-append "Combine these sub-results into one answer for: " task "\n"
         (string-join (map (lambda (i r) (format #f "~a. ~a" (+ i 1) r))
                           (iota (length results)) results) "\n"))
       "Be concise."))

;; ── RLM Main (the fractal tree) ──

(define (rlm task . opts)
  (let ((depth (if (null? opts) 0 (car opts))))
    (if (>= depth MAX-DEPTH)
        (llm task)
        (let loop ((retry 0))
          (display (format #f "[rlm d=~a r=~a] ~a\n" depth retry
                    (substring task 0 (min 50 (string-length task)))))
          (force-output)
          (let ((result (try-solve task)))
            (cond
             ((eq? (car result) 'ok)
              (display (format #f "[rlm d=~a] ■ BLACK ✓\n" depth))
              (force-output)
              (let ((vals (cdr result)))
                (if (null? vals) "done" (last vals))))
             ((< retry MAX-RETRIES)
              (display (format #f "[rlm d=~a] ✗ ~a\n" depth (cdr result)))
              (force-output)
              (loop (+ retry 1)))
             (else
              (display (format #f "[rlm d=~a] ⟳ SPLIT\n" depth))
              (force-output)
              (let* ((subs (decompose task))
                     (results (map (lambda (t) (rlm t (+ depth 1))) subs)))
                (synthesize task results)))))))))

;; Exposed to generated code
(define-syntax-rule (assert expr) (unless expr (error "assert failed")))
(define-syntax-rule (final val) val)

;; CLI
(when (> (length (command-line)) 1)
  (let ((task (string-join (cdr (command-line)) " ")))
    (display "═══ Fractal RLM — Guile Scheme ═══\n")
    (display (format #f "Task: ~a\nModel: ~a\n\n" task MODEL))
    (force-output)
    (let ((result (rlm task)))
      (display "\n═══ RESULT ═══\n")
      (display result)
      (newline)
      (display (format #f "Tokens: ~a | Calls: ~a\n" rlm-tokens rlm-calls)))))
