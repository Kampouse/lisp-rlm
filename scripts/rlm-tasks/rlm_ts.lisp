;; rlm_ts.lisp — TS-surface helpers for @ts task variants.
;; Loaded AFTER rlm_runtime.lisp (and policy/q-table/custom-actions).
;; The brain writes TypeScript; ts->lisp (builtin, ts_frontend) lowers it
;; to the same s-expression source the Lisp arm produces, so worlds,
;; verify, budgeting, traces and Q all stay identical. Only the surface
;; the model reads/writes differs — that is the experiment (2026-10-02).

;; --- extract code from possible markdown fences ---
;; "```ts\ncode\n```" -> "code" ; bare code passes through untouched.
(define (strip-code-fences s)
  (if (str-contains s "```")
    (let* ((parts (str-split s "```"))
           (cand (if (> (len parts) 1) (str-trim (nth parts 1)) s))
           (lines (str-split cand "\n"))
           (f (str-trim (nth lines 0))))
      (if (or (= f "ts") (= f "typescript"))
        (str-join (rest lines) "\n")
        cand))
    (str-trim s)))

;; --- TS-arm prompt assembly: gate feedback + per-task CCG memory.
;; Deliberately skips dream/cheatsheet.txt (Lisp-syntax lessons would
;; teach the wrong surface). CCG memory is tag-isolated via __trace_id
;; "@ts" suffix, so each surface accumulates its own attempt history.
(define (build-prompt-ts ctx)
  (let ((fb (rlm-get __gate_feedback))
        (mem (try (read-file (str-concat "data/rlm/ccg/" (q-task-tag) ".txt")) (catch e ""))))
    (begin
      (if (and fb (not (= fb ""))) (rlm-set __gate_feedback ""))
      (str-concat
        ctx
        (if (and fb (not (= fb ""))) fb "")
        "\n" mem "\n"))))
