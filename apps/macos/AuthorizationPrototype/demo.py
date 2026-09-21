#!/usr/bin/env python3
"""Throwaway signed-decision walkthrough. No OS rights or biometric prompts."""
import json
import pathlib
import subprocess
import tempfile
import time

root = pathlib.Path(__file__).resolve().parent
subprocess.run([str(root / 'build.sh')], check=True)
exe = str(root / '.build/authorization-probe')
with tempfile.TemporaryDirectory(prefix='pauth-', dir='/tmp') as directory:
    scratch = pathlib.Path(directory)
    socket = str(scratch / 'broker.sock')
    key = str(scratch / 'demo-key')
    children = []

    def call(*args):
        return subprocess.run([exe, *args], check=True, capture_output=True, text=True).stdout

    def start(*args):
        process = subprocess.Popen([exe, *args], stdout=subprocess.PIPE, text=True)
        children.append(process)
        return process

    def challenge():
        for _ in range(100):
            pending = json.loads(call('pending', socket))
            if pending:
                return pending[0]
            time.sleep(.05)
        raise RuntimeError('No pending challenge')

    try:
        call('enroll-demo', key)
        broker = start('serve-demo', socket, key + '.pub')
        while not pathlib.Path(socket).exists():
            if broker.poll() is not None:
                raise RuntimeError('Broker exited')
            time.sleep(.05)
        print('Software-key demonstration only; no macOS right is requested.', flush=True)
        while True:
            choice = input('[a] signed allow  [d] signed deny  [p] policy allow  [q] quit: ').strip()
            if choice == 'q':
                break
            if choice == 'p':
                broker.terminate()
                broker.wait()
                pathlib.Path(socket).unlink()
                broker = start('serve-demo', socket, key + '.pub', '--automatic')
                while not pathlib.Path(socket).exists():
                    time.sleep(.05)
                print(call('request-demo', socket).strip())
                print('Policy path complete. Restart this demo to return to human decisions.')
                break
            if choice not in ('a', 'd'):
                continue
            request = start('request-demo', socket)
            payload = challenge()
            answer = 'allow' if choice == 'a' else 'deny'
            response = call('sign-demo', key, payload, answer)
            response_path = scratch / 'response.json'
            response_path.write_text(response)
            print(call('submit', socket, str(response_path)).strip())
            print(request.communicate(timeout=5)[0].strip())
    finally:
        for process in children:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
