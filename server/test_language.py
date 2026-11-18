"""Keep packaged translations and desktop approval messages in sync."""
import re
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import language


class LanguageTests(unittest.TestCase):
    def test_catalogs_have_matching_keys_and_placeholders(self):
        expected = language.messages_for("en")
        for code in language.LANGUAGES:
            with self.subTest(code=code):
                messages = language.messages_for(code)
                self.assertEqual(expected.keys(), messages.keys())
                for key, value in messages.items():
                    self.assertTrue(value.strip(), key)
                    self.assertEqual(sorted(re.findall(r"{\w+}", expected[key])),
                                     sorted(re.findall(r"{\w+}", value)), key)

    def test_locale_tags_map_to_supported_languages(self):
        cases = {"zh_TW": "zh-TW", "zh-Hant-HK": "zh-TW", "zh-HK": "zh-TW", "zh-CN": "zh-CN", "zh_Hans_SG": "zh-CN",
                 "zh": "zh-CN", "ja_JP": "ja", "ko-KR": "ko", "en_US": "en", "fr_FR": "en", "": "en", None: "en"}
        for tag, expected in cases.items():
            with self.subTest(tag=tag):
                self.assertEqual(language.normalize(tag), expected)

    def test_translate_uses_saved_language(self):
        with patch.object(language, "saved_language", return_value="en"):
            self.assertEqual(language.t("srvCwd", path="C:/x"), "Default working folder: C:/x")
        with patch.object(language, "saved_language", return_value="ja"):
            self.assertIn("C:/x", language.t("srvCwd", path="C:/x"))

    def test_only_supported_choices_are_saved(self):
        with tempfile.TemporaryDirectory() as temp:
            with patch.object(language, "PREFERENCE", Path(temp) / "app" / "language.txt"):
                self.assertIsNone(language.saved_language())
                for code in language.LANGUAGES:
                    language.save_language(code)
                    self.assertEqual(language.saved_language(), code)
                with self.assertRaises(ValueError):
                    language.save_language("../unknown")
                self.assertEqual(language.saved_language(), "ko")
                language.PREFERENCE.write_text("bad preference", encoding="utf-8")
                self.assertIsNone(language.saved_language())

