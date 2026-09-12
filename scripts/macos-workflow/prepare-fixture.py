#!/usr/bin/env python3
"""Build the real Cocoa editor fixture without launching it."""
import argparse
from pathlib import Path
import plistlib
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('directory', type=Path)
parser.add_argument('--source', type=Path, required=True)
args = parser.parse_args()
app = args.directory.resolve() / 'WorkflowEditor.app'
binary = app / 'Contents/MacOS/LaunchProbe'
binary.parent.mkdir(parents=True, exist_ok=True)
with (app / 'Contents/Info.plist').open('wb') as file:
    plistlib.dump({'CFBundleExecutable': 'LaunchProbe',
                  'CFBundleIdentifier': 'work.flotilla.porthole.workflow-editor',
                  'CFBundleName': 'WorkflowEditor', 'CFBundlePackageType': 'APPL',
                  'NSHighResolutionCapable': True}, file)
subprocess.run(['clang', '-fobjc-arc', '-framework', 'AppKit', str(args.source.resolve()), '-o', str(binary)], check=True)
print(app)
