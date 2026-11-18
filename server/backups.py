"""Automatic backups for files GeminiForge writes or edits, with restore.

Every write_file / edit_file first copies the original file (or notes that it
did not exist) into GeminiForge/backups/<backup_id>/. restore_backup puts it
back, after backing up the current version, so a restore can itself be undone.

Command line (also used by restore.bat):
    python backups.py              list recent backups
    python backups.py list [N]     list the last N backups
    python backups.py restore ID   restore one backup
"""
import json
import re
import shutil
import sys
import threading
import time
from pathlib import Path

from language import t as tr

ROOT = Path(__file__).resolve().parent.parent / "backups"
MAX_FILE_BYTES = 50 * 1024 * 1024   # larger files are not copied
KEEP_DAYS = 30
KEEP_COUNT = 5000
_LOCK = threading.Lock()
_last_id = ""


def _new_id():
    global _last_id
    base = time.strftime("%Y%m%d-%H%M%S")
    n = 1
    bid = f"{base}-{n:03d}"
    while bid <= _last_id or (ROOT / bid).exists():
        n += 1
        bid = f"{base}-{n:03d}"
    _last_id = bid
    return bid


def snapshot(path, tool):
    """Back up `path` before it changes. Returns (backup_id, note)."""
    p = Path(path)
    with _LOCK:
        ROOT.mkdir(parents=True, exist_ok=True)
        bid = _new_id()
        d = ROOT / bid
        d.mkdir()
        meta = {"id": bid, "time": time.strftime("%Y-%m-%d %H:%M:%S"), "tool": tool,
                "path": str(p), "existed": p.is_file(), "copied": False}
        if p.is_file():
            size = p.stat().st_size
            meta["size"] = size
            if size <= MAX_FILE_BYTES:
                shutil.copy2(p, d / "file")
                meta["copied"] = True
        (d / "meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=1), encoding="utf-8")
        _prune()
    if meta["existed"] and not meta["copied"]:
        return bid, tr("bkTooLarge", mb=MAX_FILE_BYTES // 1024 // 1024)
    return bid, tr("bkSaved", id=bid)


def _entries():
    if not ROOT.is_dir():
        return []
    out = []
    for d in sorted(ROOT.iterdir(), reverse=True):
        m = d / "meta.json"
        if d.is_dir() and m.is_file():
            try:
                out.append(json.loads(m.read_text(encoding="utf-8")))
            except (OSError, ValueError):
                continue
    return out


def _prune():
    dirs = sorted((d for d in ROOT.iterdir() if d.is_dir()), reverse=True)
    cutoff = time.strftime("%Y%m%d", time.localtime(time.time() - KEEP_DAYS * 86400))
    for i, d in enumerate(dirs):
        if i >= KEEP_COUNT or d.name[:8] < cutoff:
            shutil.rmtree(d, ignore_errors=True)


def list_backups(limit=20, path=None):
    items = _entries()
    if path:
        key = str(path).replace("\\", "/").lower()
        items = [e for e in items if key in e["path"].replace("\\", "/").lower()]
    items = items[:max(1, min(int(limit), 200))]
    if not items:
        return tr("bkNone")
    lines = []
    for e in items:
        state = tr("bkWasMissing") if not e["existed"] else ("" if e["copied"] else tr("bkNotCopied"))
        lines.append(f"{e['id']}  {e['time']}  {e['tool']:<14} {e['path']}  {state}".rstrip())
    return tr("bkHeader") + "\n" + "\n".join(lines)


def restore(backup_id):
    backup_id = str(backup_id).strip()
    if not re.fullmatch(r"\d{8}-\d{6}-\d{3,}", backup_id):
        raise ValueError(tr("bkBadId", id=backup_id))
    d = ROOT / backup_id
    m = d / "meta.json"
    if not m.is_file():
        raise ValueError(tr("bkNotFound", id=backup_id))
    meta = json.loads(m.read_text(encoding="utf-8"))
    p = Path(meta["path"])
    if meta["existed"] and not meta["copied"]:
        raise ValueError(tr("bkNoContent", id=backup_id))
    undo_id, _ = snapshot(p, f"restore {backup_id}")
    if meta["existed"]:
        p.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(d / "file", p)
        msg = tr("bkRestored", path=p, time=meta["time"])
    elif p.is_file():
        p.unlink()
        msg = tr("bkDeleted", path=p)
    else:
        msg = tr("bkNothing", path=p)
    return tr("bkUndo", msg=msg, id=undo_id)


def main(argv):
    cmd = argv[1] if len(argv) > 1 else "list"
    try:
        if cmd == "list":
            print(list_backups(int(argv[2]) if len(argv) > 2 else 20))
        elif cmd == "restore" and len(argv) > 2:
            print(restore(argv[2]))
        else:
            print(__doc__)
            return 2
    except ValueError as e:
        print(e)
        return 1
    return 0


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    sys.exit(main(sys.argv))
