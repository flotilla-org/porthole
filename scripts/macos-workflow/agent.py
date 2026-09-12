#!/usr/bin/env python3
"""Run the chosen agent with a pre-provisioned Porthole token, without printing it."""
import json
import os
from pathlib import Path
import sys

root = Path(sys.argv[1]).resolve()
state = json.loads((root / 'state.json').read_text())
identity = json.loads((root / 'identity.json').read_text())
os.environ['PORTHOLE_AGENT_TOKEN'] = identity['token']
os.environ['PATH'] = str(Path.home() / '.local/bin') + ':/opt/homebrew/bin:' + os.environ['PATH']
os.chdir(root)
command = state['agent_command']
os.execvp(command[0], command)
