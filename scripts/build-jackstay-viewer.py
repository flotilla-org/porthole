#!/usr/bin/env python3
"""Build the reference viewer from the exact Jackstay revision Cargo resolves."""
import json
import platform
from pathlib import Path
import subprocess
import sys


def main():
    system = platform.system()
    if system not in ('Darwin', 'Linux'):
        sys.exit('The SDL reference viewer supports macOS and Linux only')
    repo = Path(__file__).resolve().parent.parent
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--locked'], cwd=repo))
    packages = [p for p in metadata['packages'] if p['name'] == 'jackstay']
    if len(packages) != 1:
        sys.exit('Expected exactly one resolved Jackstay package')
    manifest = Path(packages[0]['manifest_path'])
    source_root = manifest.parent.parent.parent
    target = Path(metadata['target_directory'])
    library_target = target / 'jackstay'
    build = ['cargo', 'build', '--manifest-path', str(manifest), '--locked',
             '--target-dir', str(library_target)]
    if system == 'Darwin':
        build += ['--features', 'backend-macos']
    subprocess.run(build, cwd=repo, check=True)
    library = library_target / 'debug' / ('libjackstay.dylib' if system == 'Darwin' else 'libjackstay.so')
    viewer_target = target / 'capture-viewer-sdl'
    # Each Git revision has a new source directory; discard the previous CMake cache.
    subprocess.run(['cmake', '--fresh', '-S', str(source_root / 'tools' / 'capture-viewer-sdl'),
                    '-B', str(viewer_target), '-DJACKSTAY_LIB=' + str(library),
                    '-DJACKSTAY_INCLUDE_DIR=' + str(manifest.parent / 'include')], check=True)
    subprocess.run(['cmake', '--build', str(viewer_target)], check=True)
    print(viewer_target / 'capture-viewer-sdl')


if __name__ == '__main__':
    main()
