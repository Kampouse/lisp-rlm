;; DREAM TACTICS — the Darwin Gödel mutation: the agent rewrites its own
;; self-advice (scripts/rlm-tasks/tactics.txt), which build-prompt injects
;; into EVERY future prompt. Grounded in: honest scoreboard + TRUE builtin
;; signatures (docs.txt, machine-extracted) + its recent falsifiable
;; lessons. Selection happens next cycles via rlm-dream.py evolve_stamp
;; (champion kept, drift reverted). Mutation whitelist: tactics.txt ONLY.
(load-file "rlm_runtime.lisp")
(load-file "scripts/rlm-tasks/policy.lisp")

(define (read-file-or f d)
  (try (read-file f) (catch e d)))

(define SCOREBOARD (read-file-or "data/rlm/dream/scoreboard.txt" "no data"))
(define DOCS (read-file-or "data/rlm/dream/docs.txt" ""))
(define OLD (read-file-or "scripts/rlm-tasks/tactics.txt" ""))
(define CORRECTIONS (read-file-or "data/rlm/dream/facts-corrections.txt" ""))

(define PROPOSAL
  (llm (str-concat
    "You are the DREAM layer of a Lisp agent. You may rewrite the agent's TACTICS —\n"
    "self-advice injected into every one of its future prompts. This is your only\n"
    "self-modification power; make it count.\n\n"
    "HONEST SCOREBOARD (recent worlds):\n" SCOREBOARD "\n\n"
    "TRUE BUILTIN SIGNATURES (the ground truth — never contradict these):\n"
    (str-substring DOCS 0 1500) "\n\n"
    "CURRENT TACTICS:\n" OLD "\n\n"
    (if (> (str-length CORRECTIONS) 0)
      (str-concat "YOUR RECENT WRONG BELIEFS (machine-verified — you MUST correct these in the new tactics):\n" CORRECTIONS "\n\n")
      "")
    "Rewrite the tactics file. Rules:\n"
    "- 4-8 bullet lines, each starting with '- '\n"
    "- 100-600 characters total\n"
    "- every signature claim must match the TRUE SIGNATURES above verbatim\n"
    "- target the WEAKEST tasks on the scoreboard with concrete moves\n"
    "- keep what works, cut what doesn't\n"
    "Output ONLY the file content, starting with the line: # tactics genN\n")))

(define CLEAN
  (let ((s (str-trim PROPOSAL)))
    (if (str-contains s "# tactics")
      s
      (str-concat "# tactics (dream)\n" s))))

(define LEN (str-length CLEAN))
(define OK (and (> LEN 100) (< LEN 700)))

(if OK
  (begin
    (write-file "scripts/rlm-tasks/tactics.txt" (str-concat CLEAN "\n"))
    (println "RLMDUMP tactics=rewritten")
    (println (str-concat "RLMDUMP tactics_len=" (to-string LEN))))
  (begin
    (println (str-concat "RLMDUMP tactics=rejected_len_" (to-string LEN)))
    (println (str-substring PROPOSAL 0 200))))
