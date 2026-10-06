#!/usr/bin/env python3
"""Focused stdlib-only checks: python3 scripts/test-replay-unlocks.py."""
import contextlib
import copy
import io
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest

RIG = runpy.run_path(str(Path(__file__).with_name("replay-unlocks")))


class ReplayTests(unittest.TestCase):
    def request(self):
        return {"moves": [{"item": {"range": {"start_byte": 0, "end_byte": 4}}}]}

    def value(self):
        return {"status": "complete", "truncation_reasons": ["diagnostic_count", "trivia_scope"],
                "counts": {"selected_items": 1, "omissions": {"decisions": 299}}, "coverage": {},
                "plan": {"applicable": False, "integrity": {"semantic": "not_performed"},
                         "decision_groups": [
                             {"reason": "member_or_constructor_unproved", "category": "visibility_context",
                              "blocks_applicability": True, "count": 300},
                             {"reason": "removal_gap_choice", "category": "trivia_ownership",
                              "blocks_applicability": False, "count": 1}]}}

    def test_groups_count_occurrences_not_compact_exemplars(self):
        result = RIG["measure"](self.value(), self.request())
        self.assertEqual(result["blockers"], {"member/constructor": 300})
        self.assertEqual(result["selected_items"], 1)
        self.assertEqual(result["selected_bytes"], 4)
        self.assertEqual(result["applicable_item_estimate"], 0)

    def test_all_segments_and_unknown_fallback(self):
        pairs = [("member_or_constructor_unproved", "visibility_context", "member/constructor"),
                 ("macro_context_unexamined", "macro_dependency", "macro"),
                 ("external_or_missing_binding", "scope_dependency", "external-binding"),
                 ("conditional_or_inherited_context", "attribute_context", "conditional/derive"),
                 ("conditional_or_inherited_context", "module_context", "cfg-test/module-context"),
                 ("module_chain_failure", "scope_dependency", "cfg-test/module-context"),
                 ("glob_binding_unproved", "glob_dependency", "glob"),
                 ("future_unknown_reason", "new_category", "other")]
        for reason, category, expected in pairs:
            with self.subTest(reason=reason, category=category):
                self.assertEqual(RIG["segment"]({"reason": reason, "category": category}), expected)

    def test_partial_work_never_counts_as_a_blocked_zero(self):
        for reasons in (["response_bytes"], ["planning_deadline"], ["inventory_work_limit"]):
            value = self.value()
            value["truncation_reasons"] = reasons
            with self.assertRaisesRegex(ValueError, "incomplete"):
                RIG["measure"](value, self.request())
        value["truncation_reasons"] = []
        value["status"] = "partial"
        with self.assertRaisesRegex(ValueError, "incomplete"):
            RIG["measure"](value, self.request())

    def test_applicable_estimates_require_complete_artifacts(self):
        value = self.value()
        value["plan"].update(applicable=True, decision_groups=[], edits=[], created_files=[], patch="patch")
        result = RIG["measure"](value, self.request())
        self.assertEqual(result["applicable_item_estimate"], 1)
        self.assertEqual(result["applicable_byte_estimate"], 4)
        value["plan"]["patch"] = None
        with self.assertRaisesRegex(ValueError, "missing artifacts"):
            RIG["measure"](value, self.request())

    def test_summary_has_positive_unlock_and_regression(self):
        base = RIG["measure"](self.value(), self.request())
        yes = copy.deepcopy(base)
        yes["applicable"] = True
        result = [{"id": "positive", "codebase": "sample", "runs": dict(zip(
            RIG["COLUMNS"], [base, yes, yes, base]))}]
        binaries = [{"path": "fake", "version": "test", "sha256": "0"}] * 2
        with contextlib.redirect_stdout(io.StringIO()) as output:
            RIG["report"](result, binaries)
        self.assertIn("newly off/on=[1, 0]; regressed off/on=[0, 1]", output.getvalue())

    def test_snapshot_detects_mutation_creation_deletion_and_ignored_destination(self):
        with tempfile.TemporaryDirectory(prefix="replay-snapshot-") as directory:
            repo = Path(directory)
            subprocess.run(["git", "init", "-q", str(repo)], check=True)
            (repo / "lib.rs").write_text("fn a() {}\n")
            (repo / ".gitignore").write_text("new.rs\n")
            request = {"repo_path": str(repo), "crate_root": "lib.rs", "paths": ["."], "moves": [
                {"item": {"path": "lib.rs"}, "destination": {"path": "new.rs", "parent_path": "lib.rs"}}]}
            before = RIG["snapshot"](request)
            self.assertIsNone(before[0]["new.rs"])
            (repo / "lib.rs").write_text("fn b() {}\n")
            self.assertNotEqual(RIG["snapshot"](request), before)
            (repo / "lib.rs").write_text("fn a() {}\n")
            (repo / "new.rs").write_text("fn moved() {}\n")
            self.assertNotEqual(RIG["snapshot"](request), before)
            (repo / "new.rs").unlink()
            (repo / "lib.rs").unlink()
            self.assertNotEqual(RIG["snapshot"](request), before)

    def test_unsafe_paths_and_symlinks_rejected(self):
        with tempfile.TemporaryDirectory(prefix="replay-path-") as directory:
            repo = Path(directory)
            for name in ("../escape.rs", "/escape.rs", ".git/config"):
                with self.assertRaises(ValueError):
                    RIG["relative_file"](repo, name)
            (repo / "link.rs").symlink_to(repo / "absent.rs")
            with self.assertRaisesRegex(ValueError, "symlink"):
                RIG["relative_file"](repo, "link.rs")

    def test_rpc_stdin_open_notifications_errors_eof_and_timeout(self):
        # This tiny server replies only while stdin remains open. Real server
        # correctness/positive artifacts are covered separately by corpus replay.
        with tempfile.TemporaryDirectory(prefix="replay-rpc-") as directory:
            server = Path(directory) / "server"
            server.write_text('''#!/usr/bin/env python3
import json
import sys
import time
for line in sys.stdin:
    message = json.loads(line)
    if 'id' not in message:
        continue
    method = message['method']
    if method == 'stall':
        time.sleep(5)
    if method == 'eof':
        break
    print(json.dumps({'jsonrpc': '2.0', 'method': 'notifications/progress'}), flush=True)
    if method == 'fail':
        reply = {'error': {'code': -1, 'message': 'deliberate'}}
    elif method == 'tools/call':
        value = {'error': None, 'echo': message['params']['arguments']}
        reply = {'result': {'isError': False, 'structuredContent': value,
                            'content': [{'type': 'text', 'text': json.dumps(value)}]}}
    else:
        reply = {'result': {}}
    print(json.dumps({'jsonrpc': '2.0', 'id': message['id'], **reply}), flush=True)
''')
            server.chmod(0o755)
            with RIG["Client"](server, 2) as client:
                self.assertEqual(client.call("move_item", {"assume_standard_prelude": True})["echo"],
                                 {"assume_standard_prelude": True})
                with self.assertRaisesRegex(ValueError, "JSON-RPC failed"):
                    client.rpc("fail", {})
            with RIG["Client"](server, 2) as client:
                with self.assertRaisesRegex(ValueError, "server exited"):
                    client.rpc("eof", {})
            with RIG["Client"](server, 0.1) as client:
                with self.assertRaises(TimeoutError):
                    client.rpc("stall", {})
            self.assertIsNotNone(client.process.poll())


if __name__ == "__main__":
    unittest.main()
