"""Cheap isolated contract tests; never compile or launch the game."""
import importlib.util
import json
import os
import shutil
import subprocess
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('gate', Path(__file__).with_name('gate.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class RatchetTests(unittest.TestCase):
    def test_loc_new_growth_and_shrink(self):
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory) / 'a.rs'
            with patch.object(gate, 'load', return_value={'limit': 800, 'files': {str(p): 810}}):
                p.write_text('\n' * 810)
                gate.loc([str(p)])
                p.write_text('\n' * 811)
                with self.assertRaises(ValueError):
                    gate.loc([str(p)])
                p.write_text('\n' * 799)
                gate.loc([str(p)])
            with patch.object(gate, 'load', return_value={'limit': 800, 'files': {}}):
                p.write_text('\n' * 800)
                gate.loc([str(p)])
                p.write_text('\n' * 801)
                with self.assertRaises(ValueError):
                    gate.loc([str(p)])

    def test_loc_skips_a_listed_generated_file(self):
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory) / 'generated.rs'
            p.write_text('\n' * 900)
            with patch.object(gate, 'load', return_value={'limit': 800, 'files': {}}), \
                    patch.object(gate, 'GENERATED', {str(p)}):
                gate.loc([str(p)])

    def test_crap_changed_file_loses_exception_and_missing_report_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory) / 'a.rs'
            p.write_text('fn f() {}')
            baseline = {'files': {os.path.relpath(p): {'sha256': gate.digest(p), 'functions': {'f': [20]}}}}
            report = Path(directory) / 'report.json'
            entry = {'file': str(p), 'function': 'f', 'line': 1, 'crap': 20}
            report.write_text(json.dumps({'entries': [entry]}))
            with patch.object(gate, 'load', side_effect=lambda _: json.loads(json.dumps(baseline))):
                gate.crap(report)
                p.write_text('fn f() { if true {} }')
                with self.assertRaises(ValueError):
                    gate.crap(report)
                entry['crap'] = 10
                report.write_text(json.dumps({'entries': [entry]}))
                gate.crap(report)
                report.write_text('{"entries":[]}')
                with self.assertRaises(ValueError):
                    gate.crap(report)

    def test_clippy_exact_warning_multiplicity_and_changed_source(self):
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory) / 'a.rs'
            p.write_text('fn f() {}')
            message = {'level': 'error', 'code': {'code': 'clippy::test'}, 'message': 'legacy',
                       'spans': [{'file_name': str(p), 'is_primary': True}]}
            diagnostic = json.dumps({'reason': 'compiler-message', 'message': message})
            key = json.dumps([str(p), 'clippy::test', 'legacy', gate.digest(p)], separators=(',', ':'))
            report = Path(directory) / 'report.json'
            with patch.object(gate, 'load', return_value={'diagnostics': [key]}):
                report.write_text(diagnostic)
                gate.clippy(report)
                report.write_text(diagnostic + '\n' + diagnostic)
                with self.assertRaises(ValueError):
                    gate.clippy(report)
                report.write_text(diagnostic)
                p.write_text('fn changed() {}')
                with self.assertRaises(ValueError):
                    gate.clippy(report)


    def test_hook_uses_staged_bytes_not_unstaged_fix(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tooling = root / 'scripts/quality'
            tooling.mkdir(parents=True)
            for name in ('pre-commit', 'gate.py', 'loc-baseline.json'):
                shutil.copy(Path(__file__).with_name(name), tooling / name)
            subprocess.run(['git', 'init', '-q', str(root)], check=True)
            source = root / 'a.rs'
            source.write_text('fn main() {}\n' * 801)
            subprocess.run(['git', 'add', '.'], cwd=root, check=True)
            # An unstaged repair must not hide the staged 801-line violation.
            source.write_text('fn main() {}\n')
            result = subprocess.run(['python3', str(tooling / 'pre-commit')], cwd=root,
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('801 > 800', result.stderr)
            self.assertEqual(source.read_text(), 'fn main() {}\n')



if __name__ == '__main__':
    unittest.main()
