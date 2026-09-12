#!/usr/bin/env python3
"""Operator-side setup and cleanup for the manual macOS GUI agent workflow.

Uses the existing local-trust identity/request/approval CLI. Does not alter TCC.
Run on the target Mac, with its installed Porthole daemon already authorized.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['start', 'status', 'approve', 'cleanup'])
    parser.add_argument('directory', type=Path)
    parser.add_argument('--cleat', type=Path)
    parser.add_argument('--agent-command', default='codex --no-alt-screen')
    args = parser.parse_args()
    root = args.directory.resolve()
    cli = str(Path.home() / '.local/bin/porthole')

    def operator(*command):
        return subprocess.check_output([cli, *command], text=True)

    def save(state):
        (root / 'state.json').write_text(json.dumps(state, indent=2) + '\n')

    if args.action == 'start':
        info = operator('info')
        for permission in ('accessibility', 'screen_recording'):
            if f'system permission {permission}: granted' not in info:
                raise RuntimeError(f'BLOCKED: missing {permission}; run porthole onboard')
        if args.cleat is None:
            parser.error('start requires --cleat pointing to a working native cleat binary')
        cleat = args.cleat.resolve()
        help_text = subprocess.check_output([str(cleat), 'launch', '--help'], text=True)
        if '--tag' not in help_text:
            raise RuntimeError('cleat is too old for this recipe')
        root.mkdir(mode=0o700, parents=True, exist_ok=False)
        # Darwin sockaddr_un has a 104-byte path field. Keep socket names short
        # even when the evidence directory lives under a long external-volume HOME.
        state = {'cleat': str(cleat), 'server': 'workflow',
                 'session': 'agent', 'runtime': tempfile.mkdtemp(prefix='p115-', dir='/tmp'), 'surfaces': [],
                 'agent_command': shlex.split(args.agent_command)}
        save(state)
        identity = json.loads(operator('agents', 'create', '--name', state['server'], '--json'))
        with os.fdopen(os.open(root / 'identity.json', os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600), 'w') as file:
            json.dump(identity, file)
    else:
        state = json.loads((root / 'state.json').read_text())
        if args.action == 'cleanup' and not state.get('terminal_launch', {}).get('surface_id'):
            parser.error('start did not record a terminal surface; inspect the failed launch and reconcile '
                         'its run-owned processes and identity before cleanup; no cleanup actions were taken')
        if not (root / 'identity.json').exists():
            raise RuntimeError('identity already removed by cleanup, or setup did not finish')
        identity = json.loads((root / 'identity.json').read_text())

    env = dict(os.environ, PORTHOLE_AGENT_TOKEN=identity['token'], CLEAT_RUNTIME_DIR=state['runtime'])

    def drive(command):
        result = subprocess.run([cli, *command], env=env, capture_output=True, text=True, timeout=35)
        if result.returncode:
            message = (result.stdout + result.stderr).replace(identity['token'], '<redacted>')
            raise RuntimeError(message)
        return result.stdout

    def approve():
        requests = json.loads(operator('agents', 'requests', '--json'))
        for request in requests:
            if request['agent_id'] == identity['agent_id'] and request['status'] == 'pending':
                print('Approving:', request['actions'], request['target'])
                operator('agents', 'approve', request['request_id'], '--duration', 'persistent', '--json')

    def cleat(*command):
        return subprocess.check_output([state['cleat'], '--server', state['server'], *command], env=env, text=True)

    if args.action == 'start':
        agent = Path(__file__).with_name('agent.py').resolve()
        wrapper = root / 'terminal.sh'
        wrapper.write_text('#!/bin/sh\nset -eu\n' +
            'export CLEAT_RUNTIME_DIR=' + shlex.quote(state['runtime']) + '\n' +
            shlex.join([state['cleat'], '--server', state['server'], 'launch', state['session'],
                        '--tag', 'project=porthole', '--tag', 'purpose=agent', '--size', '160x45',
                        '--cwd', str(root), '--cmd', shlex.join([sys.executable, str(agent), str(root)])]) + '\n' +
            'echo "Agent is running in cleat; attach from the client Mac."\n' +
            'while [ ! -e ' + shlex.quote(str(root / 'stop-terminal')) + ' ]; do /bin/sleep 1; done\n')
        command = ['launch', '--app', '/Applications/Ghostty.app', '--require-fresh-surface', '--json',
                   '--arg=--confirm-close-surface=false', '--arg=-e', '--arg=/bin/sh', '--arg=' + str(wrapper)]
        # The new identity has no grants: this first request must stop at auth.
        try:
            drive(command)
        except RuntimeError as error:
            if 'agent_permission_needed' not in str(error):
                raise
        else:
            raise RuntimeError('new identity unexpectedly had launch authority')
        approve()
        launch = json.loads(drive(command))
        state['surfaces'].append(launch['surface_id'])
        state['terminal_launch'] = launch
        save(state)
        print('Evidence directory:', root)
        print('Attach on target (use ssh -tt from the client):')
        print(shlex.join(['env', 'CLEAT_RUNTIME_DIR=' + state['runtime'], state['cleat'],
                          '--server', state['server'], 'attach', '--no-create', state['session']]))
    elif args.action == 'approve':
        approve()
    elif args.action == 'status':
        print(cleat('inspect', state['session']))
    else:
        # The caller must add only this run's app surface IDs to state.surfaces.
        inspection = subprocess.run([state['cleat'], '--server', state['server'],
                                     'inspect', '--json', state['session']],
                                    env=env, capture_output=True, text=True)
        if inspection.returncode == 0:
            if json.loads(inspection.stdout)['session']['state'] == 'running':
                subprocess.run([state['cleat'], '--server', state['server'], 'kill', state['session']], env=env, check=True)
        elif inspection.stderr.strip() != 'not found':
            raise RuntimeError(inspection.stderr)
        (root / 'stop-terminal').touch()
        for surface in state['surfaces']:
            if surface in state.get('closed_surfaces', []):
                continue
            command = (['wait', surface, '--condition', 'gone']
                       if surface == state['terminal_launch']['surface_id'] else ['close', surface])
            try:
                drive(command)
            except RuntimeError as error:
                if 'system_permission_needed' in str(error):
                    raise RuntimeError('BLOCKED: ' + str(error)) from error
                if 'surface_dead' in str(error):
                    continue
                if 'agent_permission_needed' not in str(error):
                    raise
                approve()
                drive(command)
            state.setdefault('closed_surfaces', []).append(surface)
            save(state)
        for recording in Path(state['runtime']).rglob('*.cast'):
            destination = root / 'recordings' / recording.relative_to(state['runtime'])
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(recording, destination)
        operator('agents', 'revoke', identity['agent_id'])
        (root / 'identity.json').unlink()
        print('Agent revoked; test surfaces closed; cleat recording retained.')


if __name__ == '__main__':
    main()
