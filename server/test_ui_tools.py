"""Regression checks for window targeting; no desktop input is sent."""
import asyncio
import threading
import unittest
from concurrent.futures import ThreadPoolExecutor
from unittest.mock import MagicMock, Mock, patch

import server as gf


class WindowTargetingTests(unittest.TestCase):
    def setUp(self):
        self.old_ui = gf.STATE["ui"]
        self.info = {"hwnd": 12, "title": "Calculator", "pid": 30,
                     "type": "ButtonControl", "name": "One", "x": 50, "y": 60}
        gf.STATE["ui"] = {1: dict(self.info), 2: {**self.info, "name": "Two"}}

    def tearDown(self):
        gf.STATE["ui"] = self.old_ui

    def test_exact_window_wins_over_browser_tab(self):
        windows = [{"hwnd": 1, "title": "Calculator automation test - Chrome", "hidden": False},
                   {"hwnd": 12, "title": "Calculator", "hidden": False}]
        with patch.object(gf, "_top_windows", return_value=windows):
            self.assertEqual(gf._find_window("Calculator")["hwnd"], 12)

    def test_window_id_disambiguates_identical_titles(self):
        windows = [{"hwnd": 11, "title": "Calculator"}, {"hwnd": 12, "title": "Calculator"}]
        with patch.object(gf, "_top_windows", return_value=windows):
            self.assertEqual(gf._find_window(window_id=12)["hwnd"], 12)
            with self.assertRaises(gf.ToolError):
                gf._find_window("Media Player", window_id=12)
            with self.assertRaises(gf.ToolError):
                gf._find_window(window_id=99)

    def test_empty_window_selector_is_rejected(self):
        with self.assertRaises(gf.ToolError):
            gf._find_window("")

    def test_element_must_belong_to_pinned_window(self):
        with self.assertRaises(gf.ToolError):
            gf._ui_info(name="One", window_id=99)

    def test_duplicate_exact_names_require_element_id(self):
        gf.STATE["ui"][3] = dict(self.info)
        with self.assertRaises(gf.ToolError):
            gf._ui_info(name="One")

    def test_display_text_does_not_shadow_button(self):
        gf.STATE["ui"][3] = {**self.info, "type": "TextControl"}
        self.assertEqual(gf._ui_info(name="One")[0], 1)

    def test_failed_observation_clears_cached_elements(self):
        with patch.object(gf, "_auto", side_effect=RuntimeError("UI unavailable")):
            with self.assertRaises(RuntimeError):
                gf.t_ui_elements(window_id=12)
        self.assertEqual(gf.STATE["ui"], {})

    def test_window_title_or_process_change_rejects_cached_control(self):
        def title_into_buffer(hwnd, buf, size):
            buf.value = "Media Player"
        user32 = Mock()
        user32.GetWindowTextLengthW.return_value = 20
        user32.GetWindowTextW.side_effect = title_into_buffer
        user32.IsWindow.return_value = True
        with patch.object(gf.ctypes.windll, "user32", user32), patch.object(
                gf, "_window_process_id", return_value=30):
            with self.assertRaises(gf.ToolError):
                gf._validate_ui_window(self.info)
            user32.GetWindowTextW.side_effect = lambda hwnd, buf, size: setattr(buf, "value", "Calculator")
            with patch.object(gf, "_window_process_id", return_value=31):
                with self.assertRaises(gf.ToolError):
                    gf._validate_ui_window(self.info)

    def test_missing_control_never_clicks_cached_coordinates(self):
        auto = MagicMock()
        with patch.object(gf, "_validate_ui_window", return_value="Calculator"), patch.object(
                gf, "_auto", return_value=auto), patch.object(gf, "_refind", return_value=None), patch.object(
                gf.pyautogui, "click") as click:
            with self.assertRaises(gf.ToolError):
                gf.t_click_element(name="One", window_id=12)
            click.assert_not_called()

    def test_failed_foreground_switch_is_not_reported_as_success(self):
        user32 = Mock()
        user32.IsIconic.return_value = False
        user32.GetForegroundWindow.return_value = 99
        with patch.object(gf.ctypes.windll, "user32", user32), patch.object(
                gf.pyautogui, "press"), patch.object(gf.time, "sleep"):
            with self.assertRaises(gf.ToolError):
                gf._activate({"hwnd": 12})

    def test_batch_checks_entire_plan_before_first_click(self):
        with patch.object(gf, "t_click_element") as click:
            with self.assertRaises(gf.ToolError):
                gf.t_click_elements(12, ["One", "missing"])
            click.assert_not_called()

    def test_batch_is_ordered_and_returns_last_window_text(self):
        with patch.object(gf, "_validate_ui_window"), patch.object(
                gf, "t_click_element", side_effect=["display 1", "display 12"]) as click:
            result = gf.t_click_elements(12, ["One", "Two"])
            self.assertEqual([c.kwargs["id"] for c in click.call_args_list], [1, 2])
            self.assertIn("display 12", result)

    def test_batch_stops_after_failure_without_retrying(self):
        with patch.object(gf, "_validate_ui_window"), patch.object(
                gf, "t_click_element", side_effect=["ok", gf.ToolError("gone")]) as click:
            with self.assertRaisesRegex(gf.ToolError, r"1 step\(s\) completed"):
                gf.t_click_elements(12, ["One", "Two", "One"])
            self.assertEqual(click.call_count, 2)

    def test_batch_has_a_finite_limit(self):
        for names in ([], "One", ["One"] * 17, [1]):
            with self.subTest(names=names), self.assertRaises(gf.ToolError):
                gf.t_click_elements(12, names)

    def test_ui_calls_share_one_living_thread_across_request_workers(self):
        with patch.object(gf, "_call_tool", side_effect=lambda *args: threading.get_ident()):
            with ThreadPoolExecutor(max_workers=3) as requests:
                ids = list(requests.map(lambda _: gf.call_tool("ui_elements", {}), range(6)))
        self.assertEqual(len(set(ids)), 1)
        self.assertNotEqual(ids[0], threading.get_ident())


class MCPErrorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import mcp_server
        cls.mcp_server = mcp_server

    def test_tool_error_survives_sdk_wrapper_as_execution_error(self):
        module = self.mcp_server
        tool = module.mcp._tool_manager.get_tool("read_file")
        with patch.object(module, "approve", return_value=True), patch.object(
                module.gf, "call_tool", side_effect=gf.ToolError("file does not exist")):
            with self.assertRaisesRegex(module.MCPToolError, "file does not exist"):
                asyncio.run(tool.run({"path": "missing"}, context=None))

    def test_denied_action_never_reaches_local_tool(self):
        module = self.mcp_server
        with patch.object(module, "approve", return_value=False), patch.object(module.gf, "call_tool") as call:
            with self.assertRaises(module.MCPToolError):
                module.run_tool("click_elements", {"window_id": 12, "names": ["One"]})
            call.assert_not_called()


if __name__ == "__main__":
    unittest.main()
