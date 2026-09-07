"""[GPT-6] Typed, independently checkable pre-measurement admission failures."""
import argparse
import json
import os
from pathlib import Path


class AdmissionFailure(RuntimeError):
    """Only validated pre-workload resource/deadline failures may continue a grid."""
    def __init__(self, record):
        self.record = record
        super().__init__('preload admission: ' + record['outcome'])


def counters(text):
    return {key: int(value) for key, value in (line.split() for line in text.splitlines())}


def read_records(path, closed=False):
    """Live readers defer the final unfinished line, including partial UTF-8."""
    payload = path.read_bytes() if path.exists() else b''
    if not closed: payload = payload[:payload.rfind(b'\n') + 1]
    records = [json.loads(line) for line in payload.decode('utf-8').splitlines()]
    if any(not isinstance(row, dict) for row in records): raise ValueError('startup record must be an object')
    return records


def validate_records(records, population, workers, native):
    """Permit only a valid partial prefix of the native startup protocol."""
    seen = set(); progress = {}; archive = False
    for row in records:
        kind = row.get('record_type')
        if kind == 'native-archive-verified':
            if archive or row.get('manifest') != native: raise ValueError('startup archive identity differs')
            archive = True
        elif kind == 'preload-progress':
            index = row['worker']; loaded = row['loaded_pods']
            if type(index) is not int or not 0 <= index < workers or not archive or row.get('population') != population or type(loaded) is not int or not progress.get(index, 0) <= loaded <= len(range(index, population, workers)):
                raise ValueError('invalid partial startup progress')
            progress[index] = loaded
        elif kind == 'worker-population-ready':
            index = row['worker']
            if type(index) is not int or not 0 <= index < workers or index in seen or not archive:
                raise ValueError('invalid or duplicated startup worker')
            seen.add(index); state = row['state']; expected = len(range(index, population, workers))
            if row['assigned_pods'] != expected or state['population'] != population or state['retained_pods'] != expected or state['storage_mode'] != 'native' or state['ready'] is not True or state['poisoned'] is not False:
                raise ValueError('mismatched partial worker readiness')
            counts = state['activations']
            for key, value in {'rdf_parses': 0, 'native_dataset_loads': expected, 'initial_authorizations': expected, 'journal_entries_replayed': 0, 'attempted_activations_after_ready': 0}.items():
                if counts.get(key) != value: raise ValueError('invalid partial worker activation counters')
        else:
            raise ValueError('complete readiness, error or unknown event cannot be a negative admission prefix')


def classify(record):
    """Require local OOM evidence, or a live unit at the declared startup deadline.

    An exit code, SIGKILL, a large RSS or a systemd oom-kill result alone does
    not prove the unit's memory limit caused the failure. The terminal capture
    runs inside ExecStopPost before systemd destroys the unit's cgroup.
    """
    if record.get('record_type') != 'preload-admission-outcome' or record.get('version') != 1:
        raise ValueError('unknown preload admission record')
    if record.get('storage_mode') != 'native' or record.get('ready_observed') is not False or record.get('timed_load_started') is not False or record.get('warmup_started') is not False:
        raise ValueError('admission failure must precede ready, warmup and measurement')
    unit = record['unit']; limit = record['memory_max_bytes']; terminal = record['terminal_capture']
    if type(limit) is not int or limit <= 0 or not isinstance(terminal, dict) or terminal.get('record_type') != 'preload-terminal-cgroup' or terminal.get('unit') != unit or terminal.get('errors'):
        raise ValueError('terminal cgroup evidence missing or invalid')
    path = terminal.get('cgroup_path', '')
    if 'sparq-pod-bench.slice' not in path.split('/') or not path.endswith('/' + unit):
        raise ValueError('terminal evidence belongs to another cgroup')
    group = terminal['cgroup']; events = counters(group['memory.events.local'])
    if not {'oom', 'oom_kill'} <= events.keys():
        raise ValueError('terminal OOM counters unavailable')
    if int(group['memory.max']) != limit or int(group['memory.swap.max']) != 0:
        raise ValueError('terminal memory/swap limit differs')
    cleanup = record.get('cleanup', {})
    if cleanup.get('stopped') is not True or cleanup.get('unit') != unit or cleanup.get('active_state') not in ('failed', 'inactive', 'not-found'):
        raise ValueError('owned unit cleanup not confirmed')
    snapshot = record['resources']; properties = snapshot['systemd']
    if properties.get('ControlGroup') not in ('', path):
        raise ValueError('startup snapshot belongs to another cgroup')
    if record['outcome'] == 'memory-limit':
        if terminal.get('service_result') != 'oom-kill' or events.get('oom', 0) <= 0 or events.get('oom_kill', 0) <= 0:
            raise ValueError('no confirmed local memory-limit OOM')
    elif record['outcome'] == 'startup-timeout':
        timeout = record['startup_timeout_seconds']; elapsed = record['startup_elapsed_seconds']
        if type(timeout) not in (int, float) or timeout <= 0 or type(elapsed) not in (int, float) or not timeout <= elapsed < float('inf'):
            raise ValueError('declared startup deadline not observed')
        live = snapshot['cgroup']
        live_events = counters(live['memory.events'])
        if not {'oom', 'oom_kill'} <= live_events.keys():
            raise ValueError('live OOM counters unavailable')
        if properties.get('ControlGroup') != path:
            raise ValueError('live timeout cgroup path missing')
        if properties.get('ActiveState') != 'active' or properties.get('SubState') != 'running' or int(properties.get('MainPID', '0')) <= 0 or properties.get('Result') != 'success':
            raise ValueError('startup deadline did not observe a running unit')
        if int(live['memory.max']) != limit or int(live['memory.swap.max']) != 0 or any(live_events[key] for key in ('oom', 'oom_kill')):
            raise ValueError('timeout resource evidence missing or contains an OOM')
        if terminal.get('service_result') != 'success' or any(events.get(key, 0) for key in ('oom', 'oom_kill')):
            raise ValueError('unit failed during timeout cleanup')
    else:
        raise ValueError('unrecognized preload admission outcome')
    return record['outcome']


def capture(output, unit):
    """Tiny ExecStopPost collector; it never changes limits or experimental data."""
    record = {'record_type': 'preload-terminal-cgroup', 'unit': unit,
        'service_result': os.environ.get('SERVICE_RESULT'), 'exit_code': os.environ.get('EXIT_CODE'),
        'exit_status': os.environ.get('EXIT_STATUS'), 'cgroup': {}, 'errors': []}
    try:
        path = next(line[3:] for line in Path('/proc/self/cgroup').read_text().splitlines() if line.startswith('0::'))
        if 'sparq-pod-bench.slice' not in path.split('/') or not path.endswith('/' + unit):
            raise ValueError('collector is outside selected unit')
        record['cgroup_path'] = path
        for name in ('memory.current', 'memory.peak', 'memory.max', 'memory.swap.max', 'memory.events', 'memory.events.local', 'memory.stat', 'cpu.stat', 'io.stat'):
            record['cgroup'][name] = (Path('/sys/fs/cgroup') / path.lstrip('/') / name).read_text()
    except (OSError, ValueError, StopIteration) as error:
        record['errors'].append(str(error))
    # Never replace earlier evidence if systemd unexpectedly runs the hook twice.
    with output.open('x') as stream: json.dump(record, stream, sort_keys=True); stream.write('\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--capture', type=Path, required=True); parser.add_argument('--unit', required=True)
    args = parser.parse_args(); capture(args.capture, args.unit)
