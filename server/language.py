"""Shared language preference for the local server, MCP server and approval dialog."""
import ctypes
import json
import locale
import os
import tempfile
from functools import lru_cache
from pathlib import Path

LANGUAGES = ("en", "zh-TW", "zh-CN", "ja", "ko")
PREFERENCE = Path(os.environ.get("LOCALAPPDATA") or Path.home()) / "GeminiForge" / "language.txt"
CATALOGS = Path(__file__).resolve().parents[1] / "extension" / "_locales"


def normalize(tag):
    """Map a locale tag such as zh_Hant_TW or ja-JP to a supported language (English otherwise)."""
    parts = str(tag or "").lower().replace("_", "-").split("-")
    if parts[0] == "zh":
        if "hant" in parts:
            return "zh-TW"
        if "hans" in parts:
            return "zh-CN"
        return "zh-TW" if any(p in ("tw", "hk", "mo") for p in parts) else "zh-CN"
    return parts[0] if parts[0] in LANGUAGES else "en"


def saved_language():
    try:
        value = PREFERENCE.read_text(encoding="utf-8").strip()
    except (OSError, UnicodeError):
        return None
    return value if value in LANGUAGES else None


def system_language():
    """The Windows display language, falling back to the process locale."""
    try:
        lcid = ctypes.windll.kernel32.GetUserDefaultUILanguage()
        return normalize(locale.windows_locale.get(lcid))
    except (AttributeError, OSError):
        return normalize(locale.getlocale()[0])


def current_language():
    return saved_language() or system_language()


def save_language(value):
    if value not in LANGUAGES:
        raise ValueError("Unsupported language")
    PREFERENCE.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=PREFERENCE.parent, delete=False) as temp:
        temp.write(value.encode("utf-8"))
    try:
        os.replace(temp.name, PREFERENCE)
    finally:
        Path(temp.name).unlink(missing_ok=True)


@lru_cache(maxsize=None)
def _catalog(language):
    path = CATALOGS / language.replace("-", "_") / "messages.json"
    return {key: item["message"] for key, item in json.loads(path.read_text(encoding="utf-8")).items()}


def messages_for(language):
    return dict(_catalog(language))


def t(key, **values):
    """Translate a catalog key into the current language, filling {placeholders}."""
    return _catalog(current_language())[key].format(**values)
