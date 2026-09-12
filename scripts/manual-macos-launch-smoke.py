#!/usr/bin/env python3
"""Live launch/input/screenshot smoke through the installed, authorized daemon.

Uses the current local-trust operator CLI to grant a temporary test agent access.
Requires real macOS Accessibility and Screen Recording grants; never changes TCC.
"""
import json
import os
from pathlib import Path
import plistlib
import shutil
import signal
import subprocess
import tempfile
import time


def main():
    repo = Path(__file__).resolve().parents[1]
    cli = shutil.which('porthole')
    if not cli:
        raise RuntimeError('Install the signed Porthole bundle and CLI first')
    info = subprocess.check_output([cli, 'info'], text=True)
    for permission in ('accessibility', 'screen_recording'):
        if f'system permission {permission}: granted' not in info:
            raise RuntimeError(f'BLOCKED: missing {permission}; grant it with porthole onboard')
    root = Path(tempfile.mkdtemp(prefix='porthole-launch-smoke-'))
    app = root / 'PortholeLaunchProbe.app'
    executable = app / 'Contents/MacOS/LaunchProbe'
    executable.parent.mkdir(parents=True)
    with (app / 'Contents/Info.plist').open('wb') as file:
        plistlib.dump({'CFBundleExecutable': 'LaunchProbe', 'CFBundleIdentifier': 'work.flotilla.porthole.launch-probe',
                      'CFBundleName': 'PortholeLaunchProbe', 'CFBundlePackageType': 'APPL',
                      'NSHighResolutionCapable': True}, file)
    subprocess.run(['clang', '-fobjc-arc', '-framework', 'AppKit',
                    str(repo / 'crates/porthole-adapter-macos/tests/fixtures/launch_probe.m'),
                    '-o', str(executable)], check=True)
    identity = json.loads(subprocess.check_output([cli, 'agents', 'create', '--name', 'macos-launch-smoke', '--json']))
    env = dict(os.environ, PORTHOLE_AGENT_TOKEN=identity['token'])
    surfaces = []
    results = []
    permission_blocked = False

    def command(args):
        nonlocal permission_blocked
        for _ in range(10):
            result = subprocess.run([cli, *args], env=env, capture_output=True, text=True, timeout=30)
            if result.returncode == 0:
                return result.stdout
            message = (result.stdout + result.stderr).replace(identity['token'], '<redacted>')
            if 'system_permission_needed' in message:
                permission_blocked = True
                raise RuntimeError('BLOCKED: ' + message)
            requests = json.loads(subprocess.check_output([cli, 'agents', 'requests', '--json']))
            pending = [r for r in requests if r.get('agent_id') == identity['agent_id'] and r.get('status') == 'pending']
            if not pending:
                raise RuntimeError(message)
            for request in pending:
                subprocess.run([cli, 'agents', 'approve', request['request_id'], '--duration', 'persistent', '--json'],
                               capture_output=True, check=True)
        raise RuntimeError('authorization did not converge')

    def read_result(path, expected_text=None):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if path.exists():
                data = json.loads(path.read_text())
                if expected_text is None or data['text'] == expected_text:
                    return data
            time.sleep(0.1)
        raise RuntimeError('probe did not report the expected launch/input state')

    print('Evidence:', root, flush=True)
    try:
        for index, target in enumerate((app, app, executable)):
            result_path = root / f'result-{index}.json'
            results.append(result_path)
            args = ['launch', '--app', str(target), '--arg', str(result_path), '--arg', 'argument with spaces',
                    '--env', 'PORTHOLE_PROBE_VALUE=environment with spaces', '--require-fresh-surface', '--json']
            if target == executable:
                args += ['--cwd', str(root)]
            launch = json.loads(command(args))
            sid = launch['surface_id']
            surfaces.append(sid)
            assert launch['confidence'] == 'strong' and launch['correlation'] == 'pid_tree', launch
            data = read_result(result_path)
            assert data['environment'] == 'environment with spaces', data
            assert data['arguments'][2] == 'argument with spaces', data
            if target == executable:
                assert Path(data['cwd']).resolve() == root.resolve(), data
            text = f'Porthole native launch {index}: input received'
            command(['text', sid, text])
            data = read_result(result_path, text)
            command(['screenshot', sid, '--out', str(root / f'screenshot-{index}.png')])
            print(f'Launch {index}: PID {data["pid"]}, input and screenshot passed', flush=True)
        pids = [read_result(path)['pid'] for path in results]
        assert len(set(pids)) == 3, pids
        print('MACOS LAUNCH SMOKE PASSED', flush=True)
    finally:
        for sid in surfaces:
            if permission_blocked:
                break
            try:
                command(['close', sid])
            except (RuntimeError, subprocess.SubprocessError) as error:
                print('Cleanup close failed:', error, flush=True)
        # A launch can create the fixture before correlation fails. Only kill
        # PIDs still executing this run's unique fixture path, never other apps.
        for path in results:
            if path.exists():
                pid = json.loads(path.read_text())['pid']
                process = subprocess.run(['/bin/ps', '-o', 'comm=', '-p', str(pid)], capture_output=True, text=True)
                if process.returncode == 0 and Path(process.stdout.strip()).resolve() == executable.resolve():
                    try:
                        os.kill(pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
        subprocess.run([cli, 'agents', 'revoke', identity['agent_id']], capture_output=True, check=True)
        print('Temporary test agent revoked', flush=True)


if __name__ == '__main__':
    main()
