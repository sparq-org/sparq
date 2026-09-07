"""[GPT-6] Resource admission is typed evidence, never a generic startup exception."""
import copy
import importlib.util
from pathlib import Path
import unittest
import tempfile

SPEC = importlib.util.spec_from_file_location('admission_tested', Path(__file__).with_name('preload_admission.py'))
A = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(A)


def admission(outcome='memory-limit', memory=128):
    unit = 'sparq-pod-preloaded-fixture.service'
    path = '/sparq.slice/sparq-pod.slice/sparq-pod-bench.slice/' + unit
    events = 'oom 1\noom_kill 1\n' if outcome == 'memory-limit' else 'oom 0\noom_kill 0\n'
    return {'record_type': 'preload-admission-outcome', 'version': 1, 'outcome': outcome,
        'unit': unit, 'population': 1, 'storage_mode': 'native', 'memory_max_bytes': memory * 1024**3,
        'startup_timeout_seconds': 100, 'startup_elapsed_seconds': 101, 'ready_observed': False,
        'timed_load_started': False, 'warmup_started': False,
        'cleanup': {'unit': unit, 'stopped': True, 'active_state': 'inactive'},
        'resources': {'systemd': {'ControlGroup': path, 'ActiveState': 'active', 'SubState': 'running', 'MainPID': '42', 'Result': 'success'},
                      'cgroup': {'memory.max': str(memory * 1024**3), 'memory.swap.max': '0', 'memory.events': events}},
        'terminal_capture': {'record_type': 'preload-terminal-cgroup', 'unit': unit, 'cgroup_path': path,
            'service_result': 'oom-kill' if outcome == 'memory-limit' else 'success', 'errors': [],
            'cgroup': {'memory.max': str(memory * 1024**3), 'memory.swap.max': '0', 'memory.events.local': events}}}


class AdmissionEvidenceTests(unittest.TestCase):
    def test_closed_partial_log_is_strict_while_live_partial_utf8_is_pending(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'server.log'
            log.write_bytes(b'{"record_type":"preload-progress"}\n{"partial":"\xc3')
            self.assertEqual(A.read_records(log), [{'record_type': 'preload-progress'}])
            with self.assertRaises(UnicodeDecodeError): A.read_records(log, closed=True)
            log.write_bytes(b'{broken}\n')
            with self.assertRaises(ValueError): A.read_records(log)

    def test_only_confirmed_local_oom_and_live_declared_timeout_are_typed(self):
        for outcome in ('memory-limit', 'startup-timeout'):
            self.assertEqual(A.classify(admission(outcome)), outcome)
        record = admission(); record['resources']['systemd']['ControlGroup'] = ''
        self.assertEqual(A.classify(record), 'memory-limit')  # Removed cgroup, retained terminal proof.

    def test_global_oom_unknown_exit_missing_capture_or_cleanup_are_not_admitted(self):
        for mutate in (
                lambda r: r['terminal_capture']['cgroup'].update({'memory.events.local': 'oom 0\noom_kill 1\n'}),
                lambda r: r['terminal_capture'].update(service_result='exit-code'),
                lambda r: r['terminal_capture'].update(errors=['collector failed']),
                lambda r: r.update(terminal_capture=None),
                lambda r: r['cleanup'].update(stopped=False),
                lambda r: r['cleanup'].update(active_state='active'),
                lambda r: r['terminal_capture'].update(cgroup_path='/another.service'),
                lambda r: r.update(ready_observed=True),
                lambda r: r.update(timed_load_started=True)):
            record = admission(); mutate(record)
            with self.assertRaises(ValueError): A.classify(record)

    def test_timeout_cannot_hide_exit_early_deadline_or_a_racing_oom(self):
        for mutate in (
                lambda r: r['resources']['systemd'].update(ActiveState='failed'),
                lambda r: r['resources']['systemd'].update(Result='exit-code'),
                lambda r: r.update(startup_elapsed_seconds=99),
                lambda r: r.update(startup_elapsed_seconds=float('nan')),
                lambda r: r['resources']['cgroup'].update({'memory.events': ''}),
                lambda r: r['terminal_capture']['cgroup'].update({'memory.events.local': ''}),
                lambda r: r['terminal_capture'].update(service_result='oom-kill')):
            record = admission('startup-timeout'); mutate(record)
            with self.assertRaises(ValueError): A.classify(record)

    def test_corrupt_or_complete_startup_cannot_be_reclassified_as_resource_failure(self):
        native = {'archive_sha256': 'a' * 64}
        rows = [{'record_type': 'native-archive-verified', 'manifest': native},
                {'record_type': 'preload-progress', 'worker': 0, 'loaded_pods': 1, 'population': 32}]
        A.validate_records(rows, 32, 16, native)
        for extra in ({'record_type': 'all-population-ready'}, {'record_type': 'server-ready'},
                      {'record_type': 'worker-preload-error'}, rows[0]):
            with self.assertRaises(ValueError): A.validate_records(rows + [extra], 32, 16, native)
        wrong = copy.deepcopy(rows); wrong[1]['loaded_pods'] = 3
        with self.assertRaises(ValueError): A.validate_records(wrong, 32, 16, native)


if __name__ == '__main__': unittest.main()
