#!/usr/bin/env python3
"""
Ralph Agent Test Harness

Mock environment for testing the Ralph agent locally.
Provides in-memory storage and a real filesystem workspace.

Usage:
    python3 test_ralph.py "status"
    python3 test_ralph.py "write /workspace/hello.py:print('hello')"
    python3 test_ralph.py "read /workspace/hello.py"
"""

import json
import os
import sys
from pathlib import Path

# Workspace directory for file operations
WORKSPACE = Path(__file__).parent / "ralph-workspace"
WORKSPACE.mkdir(exist_ok=True)

# Storage file for persistence
STORAGE_FILE = Path(__file__).parent / "ralph-storage.json"

# In-memory storage (mock OutLayer storage)
STORAGE = {}

def load_storage():
    """Load storage from disk"""
    global STORAGE
    if STORAGE_FILE.exists():
        STORAGE = json.loads(STORAGE_FILE.read_text())
    else:
        STORAGE = {}

def save_storage():
    """Save storage to disk"""
    STORAGE_FILE.write_text(json.dumps(STORAGE, indent=2))

def storage_get(key):
    """Mock storage-get: return value or None"""
    return STORAGE.get(key)

def storage_set(key, value):
    """Mock storage-set: store value"""
    STORAGE[key] = value
    save_storage()
    return "ok"

def fs_read(path):
    """Read from workspace filesystem"""
    # Normalize path
    if path.startswith("/workspace/"):
        path = path[len("/workspace/"):]
    elif path.startswith("/"):
        path = path[1:]
    
    filepath = WORKSPACE / path
    if filepath.exists():
        return filepath.read_text()
    return None

def fs_write(path, content):
    """Write to workspace filesystem"""
    # Normalize path
    if path.startswith("/workspace/"):
        path = path[len("/workspace/"):]
    elif path.startswith("/"):
        path = path[1:]
    
    filepath = WORKSPACE / path
    filepath.parent.mkdir(parents=True, exist_ok=True)
    filepath.write_text(content)
    return f"Wrote {len(content)} bytes to {path}"

def fs_list(path):
    """List directory in workspace"""
    if path.startswith("/workspace/"):
        path = path[len("/workspace/"):]
    elif path.startswith("/workspace"):
        path = path[len("/workspace"):]
    elif path.startswith("/"):
        path = path[1:]
    
    path = path.strip("/")
    dirpath = WORKSPACE / path if path else WORKSPACE
    if dirpath.exists() and dirpath.is_dir():
        files = [f.name for f in dirpath.iterdir()]
        return json.dumps(files)
    return "[]"

# Initialize with sample tasks
def boot_agent():
    """Initialize agent with sample tasks"""
    STORAGE["ralph:tasks"] = json.dumps([
        {"id": "task-1", "desc": "Create a Python hello world program", "status": "pending", "priority": 80},
        {"id": "task-2", "desc": "Write a function that computes fibonacci", "status": "pending", "priority": 60},
    ])
    STORAGE["ralph:phase"] = "idle"
    STORAGE["ralph:agent:intent"] = "ralph-agent"
    save_storage()
    return "booted-with-tasks"

def handle_command(msg):
    """Process incoming command"""
    if msg == "status":
        phase = STORAGE.get("ralph:phase", "idle")
        current = STORAGE.get("ralph:current", "")
        tasks = json.loads(STORAGE.get("ralph:tasks", "[]"))
        pending = sum(1 for t in tasks if t.get("status") == "pending")
        return json.dumps({"phase": phase, "current": current, "pending": pending, "total": len(tasks)})
    
    elif msg == "tasks":
        return STORAGE.get("ralph:tasks", "[]")
    
    elif msg == "progress":
        return STORAGE.get("ralph:progress", "")
    
    elif msg == "reset":
        STORAGE.clear()
        save_storage()
        boot_agent()
        return "Agent reset."
    
    elif msg == "run":
        STORAGE["ralph:phase"] = "idle"
        return "Running."
    
    elif msg == "boot":
        return boot_agent()
    
    elif msg.startswith("add task ") or msg.startswith("add task:"):
        # Accept both "add task:" and "add task "
        colon_idx = msg.find(":")
        if colon_idx != -1:
            desc = msg[colon_idx + 1:].strip()
        else:
            desc = msg[9:].strip()
        tasks = json.loads(STORAGE.get("ralph:tasks", "[]"))
        task_id = f"task-{len(tasks) + 1}"
        tasks.append({"id": task_id, "desc": desc, "status": "pending", "priority": 50})
        STORAGE["ralph:tasks"] = json.dumps(tasks)
        return f"added:{task_id}"
    
    elif msg.startswith("read "):
        path = msg[5:]
        content = fs_read(path)
        if content:
            return content
        return f"Not found: {path}"
    
    elif msg.startswith("write "):
        rest = msg[6:]
        if ":" not in rest:
            return "Usage: write /path:content"
        path, content = rest.split(":", 1)
        return fs_write(path.strip(), content)
    
    elif msg.startswith("list "):
        path = msg[5:]
        return fs_list(path)
    
    else:
        return f"Unknown: {msg}"

def run(input_msg):
    """Main entry point - matches ralph-agent.lisp run function"""
    if not input_msg or input_msg == "":
        # Empty input = run tick cycle (for autonomous operation)
        return "tick not implemented in test harness"
    return handle_command(input_msg)

# CLI interface
if __name__ == "__main__":
    # Load persisted storage
    load_storage()
    
    if len(sys.argv) < 2:
        print("Ralph Agent Test Harness")
        print("")
        print("Commands:")
        print("  status              - Show agent state")
        print("  tasks               - List all tasks")
        print("  progress            - Show progress log")
        print("  reset               - Clear all state")
        print("  run                 - Start processing")
        print("  boot                - Initialize with sample tasks")
        print("  add task: <desc>    - Add new task")
        print("  read /path          - Read file from workspace")
        print("  write /path:content - Write file to workspace")
        print("  list /path          - List directory")
        print("")
        print("Workspace:", WORKSPACE)
        print("")
        # Boot on first run
        if "ralph:tasks" not in STORAGE:
            boot_agent()
            print("Initialized with sample tasks.")
        print("Current state:")
        print(run("status"))
        sys.exit(0)
    
    # Boot if needed
    if "ralph:tasks" not in STORAGE:
        boot_agent()
    
    cmd = sys.argv[1]
    result = run(cmd)
    print(result)