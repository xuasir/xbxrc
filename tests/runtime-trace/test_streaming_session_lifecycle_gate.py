"""Public lifecycle gate fixtures verify successful and failing client contracts."""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT=Path(__file__).resolve().parents[2]
SCRIPT=Path(os.environ.get('STREAMING_LIFECYCLE_GATE_SCRIPT', ROOT/'.agents/skills/analyze-runtime-logs/scripts/trace_streaming_session_lifecycle_gate.py'))
FIXTURES=ROOT/'tests/fixtures/streaming-session-lifecycle'
spec=importlib.util.spec_from_file_location('streaming_lifecycle_gate',SCRIPT)
gate=importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class StreamingLifecycleGateTests(unittest.TestCase):
    def test_source_fingerprint_binding(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'trace.jsonl'
            build={'event':'runtimeBuildInfo','seq':0,'payload':{'buildFingerprint':{'git_commit_short':'02f6db4','source_fingerprint_sha256':'a'*64}}}
            path.write_text(json.dumps(build)+'\n'+(FIXTURES/'pass-recovery-stable.jsonl').read_text())
            self.assertEqual(gate.audit([path],expected_source_sha256='a'*64)['lifecycleGate'],'PASS')
            result=gate.audit([path],expected_source_sha256='b'*64)
            self.assertIn('buildSourceMismatch',{item['code'] for item in result['evidenceGaps']})

    def test_fault_matrix(self):
        for case in json.loads((FIXTURES/'cases.json').read_text()):
            files=case.get('files') or [case['file']]
            with self.subTest(case=files):
                report=gate.audit([FIXTURES/file for file in files],**case['options'])
                self.assertEqual(report['lifecycleGate'],case['gate'],report)
                codes={item['code'] for item in report['violations']+report['evidenceGaps']}
                self.assertTrue(set(case['codes']) <= codes,report)

    def test_exit_status_and_freshness(self):
        for fixture, expected in [('pass-recovery-stable.jsonl',0),('fail-close-leak.jsonl',2)]:
            process=subprocess.run([sys.executable,'-B',str(SCRIPT),str(FIXTURES/fixture)],capture_output=True,text=True)
            self.assertEqual(process.returncode,expected,process.stdout+process.stderr)
            self.assertIn('lifecycleGate',json.loads(process.stdout))
        report=gate.audit([FIXTURES/'pass-recovery-stable.jsonl'],max_age_seconds=-1)
        self.assertEqual(report['lifecycleGate'],'FAIL')
        self.assertIn('staleTrace',{item['code'] for item in report['evidenceGaps']})


if __name__=='__main__':
    unittest.main()
