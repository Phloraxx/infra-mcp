"""Small isolated adapter to Hermes' own MemoryStore; never edits Markdown directly.

Executed by the Infra gateway in the Hermes Python environment with JSON on stdin.
The memory store owns its existing .lock files, validation, atomic writes and limits.
"""
import json
import os
import sys
from pathlib import Path

def main():
    source = Path(os.environ["INFRA_HERMES_SOURCE"]).resolve(strict=True)
    if not (source / "tools" / "memory_tool.py").is_file():
        raise RuntimeError("Hermes source directory is missing its memory implementation")
    sys.path.insert(0, str(source))
    data = json.load(sys.stdin)
    if data.get("action") == "search_history":
        from tools.session_search_tool import session_search
        result = session_search(query=data["query"], limit=data.get("limit", 3))
        try:
            result = json.loads(result)
        except (TypeError, json.JSONDecodeError):
            pass
        if isinstance(result, dict) and result.get("success") is False:
            print(json.dumps(result, ensure_ascii=False, default=str))
        else:
            print(json.dumps({"success": True, "results": result}, ensure_ascii=False, default=str))
        return
    from tools.memory_tool import load_on_disk_store

    action = data["action"]
    target = data["target"]
    if target not in ("memory", "user"):
        raise ValueError("Invalid memory target")
    store = load_on_disk_store()
    if action == "read":
        result = {"success": True, "target": target,
                  "entries": store._entries_for(target),
                  "usage": store._usage(target)}
    elif action == "add":
        result = store.add(target, data["content"])
    elif action == "patch":
        result = store.patch(target, data["old_text"], data["content"])
    elif action == "remove":
        result = store.remove(target, data["old_text"])
    else:
        raise ValueError("Unsupported memory action")
    print(json.dumps(result, ensure_ascii=False, default=str))

if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        # Never print content, credentials or local filesystem paths in gateway output.
        print(json.dumps({"success": False, "error": type(exc).__name__}), flush=True)
        sys.exit(1)
