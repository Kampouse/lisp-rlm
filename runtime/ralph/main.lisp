;;; ralph-agent.lisp — Autonomous task-loop agent (Ralph pattern)
;;;
;;; Ralph pattern: Fresh context per tick, memory persists in storage.
;;; Each tick picks the next pending task, implements it, verifies, marks done.
;;; Progress log accumulates learnings for future iterations.
;;;
;;; Storage schema:
;;;   ralph:tasks     — JSON array of tasks [{id, desc, status, priority}, ...]
;;;   ralph:current   — ID of task being worked on
;;;   ralph:progress  — Accumulated learnings (like progress.txt)
;;;   ralph:phase     — State machine phase
;;;   fs:/path        — Virtual filesystem (files stored in OutLayer storage)
;;;   inbox:pending   — 1 if message waiting, 0 if not
;;;   inbox:latest    — Latest incoming message
;;;   config:chat_id  — Telegram chat ID for outbound messages

;; === Virtual Filesystem ===
;;; Files are stored as storage keys with "fs:" prefix.
;;; Directory listing tracked in "fs:index:/dir/path" as JSON array of filenames.

(define (fs-key path)
  (str-concat "fs:" path))

(define (fs-index-key dir)
  (str-concat "fs:index:" dir))

;; === Filesystem Helpers (must come first) ===

(define (fs-find-last-slash path idx last-found)
  "Find position of last / in path."
  (if (>= idx (str-length path))
    last-found
    (let ((ch (str-slice path idx (+ idx 1))))
      (if (= ch "/")
        (fs-find-last-slash path (+ idx 1) idx)
        (fs-find-last-slash path (+ idx 1) last-found)))))

(define (fs-parent-dir path)
  "Extract parent directory from path. /workspace/src/main.rs -> /workspace/src"
  (let ((last-slash (fs-find-last-slash path 0 0)))
    (if (= last-slash 0)
      "/"
      (str-slice path 0 last-slash))))

(define (fs-filename path)
  "Extract filename from path. /workspace/src/main.rs -> main.rs"
  (let ((last-slash (fs-find-last-slash path 0 0)))
    (if (= last-slash 0)
      path
      (str-slice path (+ last-slash 1) (str-length path)))))

(define (load-file-list json-str idx)
  "Parse JSON array of filenames."
  (let ((elem (json-array-get json-str idx)))
    (if (or (nil? elem) (= elem ""))
      nil
      (cons elem (load-file-list json-str (+ idx 1))))))

(define (files-to-json files acc sep)
  "Serialize list of filenames to JSON array."
  (if (nil? files)
    (str-concat acc "]")
    (let ((new-acc (str-concat acc sep "\"" (car files) "\"")))
      (files-to-json (cdr files) new-acc ","))))

(define (fs-list-contains files filename)
  "Check if filename is in list. Returns 1 if found."
  (if (nil? files)
    0
    (if (= (car files) filename)
      1
      (fs-list-contains (cdr files) filename))))

(define (fs-filter-out files filename)
  "Remove filename from list."
  (if (nil? files)
    (list)
    (if (= (car files) filename)
      (fs-filter-out (cdr files) filename)
      (cons (car files) (fs-filter-out (cdr files) filename)))))

;; === Filesystem Index Operations ===

(define (fs-add-to-index path)
  "Add file to directory index."
  (let ((dir (fs-parent-dir path))
        (filename (fs-filename path))
        (index-key (fs-index-key dir)))
    (let ((index-data (storage-get index-key)))
      (let ((files (if (or (nil? index-data) (= index-data ""))
                      (list)
                      (load-file-list index-data 0))))
        (let ((has-file? (fs-list-contains files filename)))
          (if (= has-file? 0)
            (storage-set index-key (files-to-json (cons filename files) "[" ""))
            0))))))

(define (fs-remove-from-index path)
  "Remove file from directory index."
  (let ((dir (fs-parent-dir path))
        (filename (fs-filename path))
        (index-key (fs-index-key dir)))
    (let ((index-data (storage-get index-key)))
      (if (or (nil? index-data) (= index-data ""))
        0
        (let ((files (load-file-list index-data 0)))
          (let ((filtered (fs-filter-out files filename)))
            (storage-set index-key (files-to-json filtered "[" ""))))))))

;; === Filesystem Main Operations ===

(define (fs-read path)
  "Read file content from virtual filesystem. Returns nil if not found."
  (storage-get (fs-key path)))

(define (fs-write path content)
  "Write file content to virtual filesystem. Returns 'ok'."
  (begin
    (storage-set (fs-key path) content)
    (fs-add-to-index path)
    "ok"))

(define (fs-append path content)
  "Append content to existing file. Creates if not exists."
  (let ((existing (storage-get (fs-key path))))
    (let ((new-content (if (nil? existing)
                          content
                          (str-concat existing content))))
      (begin
        (storage-set (fs-key path) new-content)
        (fs-add-to-index path)
        "ok"))))

(define (fs-exists? path)
  "Check if file exists. Returns 1 if exists, 0 if not."
  (let ((data (storage-get (fs-key path))))
    (if (nil? data) 0 1)))

(define (fs-delete path)
  "Delete file. Returns 'deleted' or 'not-found'."
  (let ((existing (storage-get (fs-key path))))
    (if (nil? existing)
      "not-found"
      (begin
        (storage-set (fs-key path) "")
        (fs-remove-from-index path)
        "deleted"))))

(define (fs-list dir)
  "List files in directory. Returns JSON array of filenames."
  (let ((index-data (storage-get (fs-index-key dir))))
    (if (or (nil? index-data) (= index-data ""))
      "[]"
      index-data)))

;; === Workspace Operations ===
;;; Convenience functions for common workspace patterns.

(define (workspace-read filename)
  "Read from /workspace/filename"
  (fs-read (str-concat "/workspace/" filename)))

(define (workspace-write filename content)
  "Write to /workspace/filename"
  (fs-write (str-concat "/workspace/" filename) content))

(define (workspace-append filename content)
  "Append to /workspace/filename"
  (fs-append (str-concat "/workspace/" filename) content))

(define (workspace-list)
  "List files in /workspace"
  (fs-list "/workspace"))

(define (workspace-delete filename)
  "Delete /workspace/filename"
  (fs-delete (str-concat "/workspace/" filename)))

;; === JSON Task Helpers ===

(define (parse-task json-str)
  (let ((id (json-get "id" json-str))
        (desc (json-get "desc" json-str))
        (status (json-get "status" json-str))
        (priority-str (json-get "priority" json-str)))
    (dict "id" id
          "desc" desc
          "status" (if (nil? status) "pending" status)
          "priority" (if (nil? priority-str) 50 (string->number priority-str)))))

(define (load-task-list json-str idx)
  (let ((elem (json-array-get json-str idx)))
    (if (or (nil? elem) (= elem ""))
      nil
      (cons (parse-task elem)
            (load-task-list json-str (+ idx 1))))))

(define (load-tasks)
  (let ((data (storage-get "ralph:tasks")))
    (if (or (nil? data) (= data ""))
      (list)
      (load-task-list data 0))))

(define (escape-json s)
  s)

(define (get-default m key default)
  (let ((v (dict/get m key)))
    (if (nil? v) default v)))

(define (task-to-json task)
  (let ((id (get-default task "id" ""))
        (desc (get-default task "desc" ""))
        (status (get-default task "status" "pending"))
        (priority (get-default task "priority" 50)))
    (str-concat
      "{\"id\":\"" id "\","
      "\"desc\":\"" (escape-json desc) "\","
      "\"status\":\"" status "\","
      "\"priority\":" (to-string priority)
      "}")))

(define (tasks-collect tasks)
  "Collect tasks into comma-separated JSON string."
  (if (nil? tasks)
    ""
    (let ((first (task-to-json (car tasks)))
          (rest (tasks-collect (cdr tasks))))
      (if (= rest "")
        first
        (str-concat first "," rest)))))

(define (tasks-to-json tasks)
  "Iterative JSON serialization — avoids recursive str-concat bug."
  (str-concat "[" (tasks-collect tasks) "]"))

(define (save-tasks tasks)
  (storage-set "ralph:tasks" (tasks-to-json tasks)))

;; === Time ===

(define (now-ms)
  0)

;; === Progress Log ===

(define (append-progress entry)
  (let ((log (storage-get "ralph:progress"))
        (timestamp (to-string (now-ms))))
    (let ((new-log (str-concat
                     (if (nil? log) "" log)
                     "\n[" timestamp "] "
                     entry)))
      (storage-set "ralph:progress" new-log)
      new-log)))

;; === Task Queries ===

(define (find-pending-task tasks)
  (if (nil? tasks)
    nil
    (let ((task (car tasks)))
      (if (!= (get-default task "status" "pending") "done")
        task
        (find-pending-task (cdr tasks))))))

(define (find-task-by-id tasks id)
  (if (nil? tasks)
    nil
    (let ((task (car tasks)))
      (if (= (get-default task "id" "") id)
        task
        (find-task-by-id (cdr tasks) id)))))

(define (count-pending tasks)
  (if (nil? tasks)
    0
    (let ((task (car tasks)))
      (if (!= (get-default task "status" "pending") "done")
        (+ 1 (count-pending (cdr tasks)))
        (count-pending (cdr tasks))))))

;; === Task State Updates ===

(define (update-task-status tasks task-id new-status)
  (if (nil? tasks)
    (list)
    (let ((task (car tasks)))
      (if (= (get-default task "id" "") task-id)
        (cons (dict/set task "status" new-status)
              (update-task-status (cdr tasks) task-id new-status))
        (cons task
              (update-task-status (cdr tasks) task-id new-status))))))

(define (mark-task-done tasks task-id result)
  (let ((updated (update-task-status tasks task-id "done")))
    (begin
      (save-tasks updated)
      (append-progress (str-concat "COMPLETED " task-id ": " result))
      updated)))

(define (mark-task-failed tasks task-id reason)
  (begin
    (append-progress (str-concat "FAILED " task-id ": " reason))
    tasks))

;; === AI Call (from multi-phase-agent pattern) ===

(define (call-ai prompt)
  "Call AI via OutLayer ai-chat host function."
  (ai-chat prompt))

;; === Verification Stub ===

(define (verify-implementation task-id)
  1)

;; === Status and Task API ===

(define (list-tasks)
  (storage-get "ralph:tasks"))

(define (get-progress)
  (storage-get "ralph:progress"))

(define (reset-agent)
  (begin
    (storage-set "ralph:tasks" "")
    (storage-set "ralph:current" "")
    (storage-set "ralph:progress" "")
    (storage-set "ralph:phase" "idle")
    "reset-complete"))

(define (status)
  (let ((phase (storage-get "ralph:phase"))
        (current (storage-get "ralph:current"))
        (tasks (load-tasks))
        (pending (count-pending (load-tasks))))
    (str-concat
      "{\"phase\":\"" (if (nil? phase) "idle" phase) "\","
      "\"current\":\"" (if (nil? current) "" current) "\","
      "\"pending\":" (to-string pending) ","
      "\"total\":" (to-string (length tasks)) "}")))

(define (add-task id desc priority)
  (let ((tasks (load-tasks)))
    (let ((new-task (dict "id" id
                          "desc" desc
                          "status" "pending"
                          "priority" (if (nil? priority) 50 priority))))
      (begin
        (save-tasks (append tasks (list new-task)))
        (str-concat "added:" id)))))

(define (add-tasks-json json-array)
  (let ((new-tasks (load-task-list json-array 0)))
    (let ((existing (load-tasks)))
      (begin
        (save-tasks (append existing new-tasks))
        (str-concat "added " (to-string (length new-tasks)) " tasks")))))

;; === Inbox / Two-Way Communication ===

(define (has-inbox?)
  (let ((p (storage-get "inbox:pending")))
    (if (= p "1") 1 0)))

(define (get-inbox)
  (let ((msg (storage-get "inbox:latest")))
    (if (nil? msg) "" msg)))

(define (clear-inbox)
  (begin
    (storage-set "inbox:latest" "")
    (storage-set "inbox:pending" "0")))

(define (get-chat-id)
  (let ((cid (storage-get "config:chat_id")))
    (if (nil? cid) "5125145880" cid)))

(define (tg-send msg)
  "Send message to configured chat via OutLayer."
  (let ((chat-id (get-chat-id)))
    (outlayer/send-telegram chat-id msg)))

(define (starts-with? str prefix)
  "Check if string starts with prefix."
  (let ((plen (str-length prefix)))
    (if (< (str-length str) plen)
      false
      (= (str-slice str 0 plen) prefix))))

(define (str-index-helper s ch idx)
  (if (>= idx (str-length s))
    -1
    (if (= (str-slice s idx (+ idx 1)) ch)
      idx
      (str-index-helper s ch (+ idx 1)))))

(define (str-index s ch)
  (str-index-helper s ch 0))

(define (handle-command msg)
  "Process incoming message. Returns response."
  (if (= msg "status")
    (status)
    (if (= msg "tasks")
      (list-tasks)
      (if (= msg "reset")
        (begin (reset-agent) "Agent reset.")
        (if (= msg "run")
          (begin (storage-set "ralph:phase" "idle") "Running.")
          (if (= msg "progress")
            (get-progress)
            (if (starts-with? msg "add task ")
              (add-task (str-concat "task-" (to-string (length (load-tasks)))) (str-slice msg 9 (str-length msg)) 50)
              (if (starts-with? msg "read ")
                (let ((path (str-slice msg 5 (str-length msg))))
                  (let ((content (fs-read path)))
                    (if (nil? content)
                      (str-concat "Not found: " path)
                      content)))
                (if (starts-with? msg "list ")
                  (fs-list (str-slice msg 5 (str-length msg)))
                  (if (starts-with? msg "write ")
                    (let ((rest (str-slice msg 6 (str-length msg))))
                      (let ((colon-pos (str-index rest ":")))
                        (if (= colon-pos -1)
                          "Usage: write /path:content"
                          (let ((path (str-slice rest 0 colon-pos)))
                            (let ((content (str-slice rest (+ colon-pos 1) (str-length rest))))
                              (begin
                                (fs-write path content)
                                (str-concat "Wrote " (to-string (str-length content)) " bytes to " path)))))))
                    (str-concat "Unknown: " msg)))))))))))

;; === Phase Handlers ===

(define (handle-idle)
  "Check inbox first, then process tasks."
  (if (= (has-inbox?) 1)
    (let ((msg (get-inbox)))
      (begin
        (clear-inbox)
        (let ((response (handle-command msg)))
          (tg-send response)
          "processed-command")))
    (let ((tasks (load-tasks)))
      (let ((task (find-pending-task tasks)))
        (if (nil? task)
          (begin
            (storage-set "ralph:phase" "done")
            "all-tasks-complete")
          (let ((task-id (get-default task "id" "")))
            (begin
              (storage-set "ralph:current" task-id)
              (storage-set "ralph:phase" "implement")
              (str-concat "picked-task:" task-id))))))))

(define (handle-implement)
  (let ((task-id (storage-get "ralph:current"))
        (tasks (load-tasks)))
    (if (or (nil? task-id) (= task-id ""))
      (begin
        (storage-set "ralph:phase" "idle")
        "no-current-task")
      (let ((task (find-task-by-id tasks task-id)))
        (if (nil? task)
          (begin
            (storage-set "ralph:phase" "idle")
            "task-not-found")
          (let ((desc (get-default task "desc" "")))
            (let ((prompt (str-concat
                            "Implement this task. Be concise. Return result summary.\n\nTask: "
                            desc)))
              (let ((result (call-ai prompt)))
                (storage-set "ralph:impl-result" result)
                (storage-set "ralph:phase" "verify")
                "implementation-complete"))))))))

(define (handle-verify)
  (let ((task-id (storage-get "ralph:current")))
    (let ((passed? (verify-implementation task-id)))
      (if (= passed? 1)
        (let ((result (storage-get "ralph:impl-result")))
          (begin
            (mark-task-done (load-tasks) task-id result)
            (storage-set "ralph:current" "")
            (storage-set "ralph:phase" "idle")
            ;; Notify on completion
            (tg-send (str-concat "✅ Task " task-id " completed:\n" result))
            "task-completed"))
        (begin
          (mark-task-failed (load-tasks) task-id "verification-failed")
          (storage-set "ralph:current" "")
          (storage-set "ralph:phase" "idle")
          ;; Notify on failure
          (tg-send (str-concat "❌ Task " task-id " failed verification"))
          "verification-failed")))))

(define (handle-done)
  (begin
    (tg-send "🎉 All tasks complete!")
    "all-tasks-done"))

;; === Main Tick ===

(define (tick)
  "Dispatch to current phase handler."
  (let ((phase (storage-get "ralph:phase")))
    (if (nil? phase)
      "no-phase-set"
      (if (= phase "idle")
        (handle-idle)
        (if (= phase "implement")
          (handle-implement)
          (if (= phase "verify")
            (handle-verify)
            (if (= phase "done")
              (handle-done)
              (str-concat "unknown-phase:" phase))))))))

(define (run input)
  "HTTP eval entry point. If input is non-empty, process as command and return response. If empty, run tick cycle."
  (if (or (nil? input) (= input ""))
    (tick)
    (handle-command input)))

;; === Default Tasks (for testing) ===

(define (boot-with-sample-tasks)
  (begin
    (storage-set "ralph:tasks" "[{\"id\":\"task-1\",\"desc\":\"Fetch current NEAR price from mainnet RPC\",\"status\":\"pending\",\"priority\":80},{\"id\":\"task-2\",\"desc\":\"Analyze price trend and summarize in 2 sentences\",\"status\":\"pending\",\"priority\":60},{\"id\":\"task-3\",\"desc\":\"Send summary to Telegram chat 5125145880\",\"status\":\"pending\",\"priority\":40}]")
    (storage-set "ralph:phase" "idle")
    (storage-set "ralph:agent:intent" "ralph-agent")
    "booted-with-tasks"))