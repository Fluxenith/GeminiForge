"""Desktop confirmation popup for GeminiForge MCP tool calls.

Usage: pythonw approve_dialog.py <tool> <args-json> <timeout-seconds>
Exit code: 0 = allow once, 2 = allow this tool for 30 minutes, 1 = deny / timeout.
"""
import json
import sys
import tkinter as tk
from language import current_language, messages_for

messages = messages_for(current_language())

tool = sys.argv[1] if len(sys.argv) > 1 else "?"
try:
    args = json.dumps(json.loads(sys.argv[2]), ensure_ascii=False, indent=2)
except Exception:
    args = sys.argv[2] if len(sys.argv) > 2 else ""
timeout = int(sys.argv[3]) if len(sys.argv) > 3 else 120
if len(args) > 2500:
    args = args[:2500] + "\n…"

result = {"code": 1}
root = tk.Tk()
root.title(messages["dialogTitle"])
root.attributes("-topmost", True)
root.configure(bg="#1e1f20")
root.resizable(False, False)

FONT = ("Segoe UI", 11)
tk.Label(root, text=messages["dialogAsk"].format(tool=tool), font=("Segoe UI", 13, "bold"),
         fg="#e3e3e3", bg="#1e1f20").pack(padx=18, pady=(16, 6), anchor="w")
box = tk.Text(root, width=64, height=min(16, max(4, args.count("\n") + 2)), font=("Consolas", 10),
              bg="#151515", fg="#e3e3e3", relief="flat", wrap="word")
box.insert("1.0", args)
box.configure(state="disabled")
box.pack(padx=18, pady=4)
count = tk.Label(root, text="", font=FONT, fg="#9aa0a6", bg="#1e1f20")
count.pack(padx=18, anchor="w")


def done(code):
    result["code"] = code
    root.destroy()


row = tk.Frame(root, bg="#1e1f20")
row.pack(padx=18, pady=(8, 16), fill="x")
tk.Button(row, text=messages["allow"], font=FONT, width=8, bg="#1a73e8", fg="white", relief="flat",
          command=lambda: done(0)).pack(side="left")
tk.Button(row, text=messages["allowTimed"], font=FONT, bg="#303134", fg="#e3e3e3", relief="flat",
          command=lambda: done(2)).pack(side="left", padx=8)
tk.Button(row, text=messages["deny"], font=FONT, width=8, bg="#5c2b29", fg="white", relief="flat",
          command=lambda: done(1)).pack(side="right")
root.protocol("WM_DELETE_WINDOW", lambda: done(1))
root.bind("<Escape>", lambda e: done(1))


def tick(left):
    if left <= 0:
        done(1)
        return
    count.configure(text=messages["countdown"].format(n=left))
    root.after(1000, tick, left - 1)


tick(timeout)
root.update_idletasks()
w, h = root.winfo_width(), root.winfo_height()
root.geometry(f"+{(root.winfo_screenwidth() - w) // 2}+{(root.winfo_screenheight() - h) // 3}")
root.lift()
root.focus_force()
root.mainloop()
sys.exit(result["code"])
