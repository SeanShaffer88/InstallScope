"""Build a per-user NSIS installer from a checksum-verified portable ZIP.

Usage: python scripts/package-installer.py PORTABLE.zip MAKENSIS.exe OUTPUT.exe
The compiler is an explicit local build dependency; nothing is downloaded.
"""
import hashlib
from pathlib import Path, PurePosixPath
import subprocess
import sys
import tempfile
import zipfile
import re


def quoted(value):
    value = str(value)
    if any(c in value for c in '\r\n\0'):
        raise ValueError('Unsafe installer path')
    return '"' + value.replace('$', '$$').replace('"', '$\\"') + '"'


def main():
    archive, compiler, output = map(lambda s: Path(s).resolve(), sys.argv[1:])
    if output.exists():
        raise ValueError('Output already exists; choose a new output path')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='SetupScope-installer-build-') as temporary:
        root = Path(temporary)
        files = []
        with zipfile.ZipFile(archive) as bundle:
            names = bundle.namelist()
            if len(names) != len(set(names)):
                raise ValueError('Duplicate ZIP entries')
            manifest = bundle.read('SetupScope/SHA256SUMS.txt').decode('utf-8-sig')
            expected = {}
            folded = set()
            for line in manifest.splitlines():
                if not line.strip():
                    continue
                digest, relative = line.split('  ', 1)
                path = PurePosixPath(relative)
                if (not path.parts or path.is_absolute() or '..' in path.parts or '\\' in relative
                        or any(c in relative for c in ':"$\r\n\0<>|?*')
                        or any(p.endswith((' ', '.')) for p in path.parts)
                        or path.parts[0].lower() == 'setupscopedata'
                        or relative.lower() in folded
                        or relative.lower() in {'install-files.nsh', 'uninstall-files.nsh',
                                                'setupscope.install-owner', 'uninstall.exe'}
                        or not re.fullmatch('[0-9a-fA-F]{64}', digest)):
                    raise ValueError('Unsafe or duplicate manifest entry')
                expected[relative] = digest
                folded.add(relative.lower())
            if len(expected) > 5000 or sum(bundle.getinfo('SetupScope/' + n).file_size
                                          for n in expected) > 512 * 1024 * 1024:
                raise ValueError('Installer payload exceeds build bounds')
            for relative, digest in expected.items():
                data = bundle.read('SetupScope/' + relative)
                if hashlib.sha256(data).hexdigest() != digest:
                    raise ValueError('Checksum mismatch: ' + relative)
                if relative.lower() in {'portable', 'portable.txt'}:
                    continue
                destination = root / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(data)
                files.append(relative)
        if not {'SetupScope.exe', 'SetupScope-cli.exe'}.issubset(files):
            raise ValueError('Both application executables are required')
        # Installed payload omits the portable marker, so give it its own manifest.
        (root / 'SHA256SUMS.txt').write_text(''.join(
            hashlib.sha256((root / name).read_bytes()).hexdigest() + '  ' + name + '\n'
            for name in sorted(files)), encoding='utf-8')
        files.append('SHA256SUMS.txt')
        install, uninstall = [], []
        directories = set()
        for number, name in enumerate(sorted(files)):
            relative = name.replace('/', '\\')
            parent = str(PurePosixPath(name).parent).replace('/', '\\')
            install += ['SetOutPath "$INSTDIR' + ('' if parent == '.' else '\\' + parent) + '"',
                        'File ' + quoted(root / name)]
            uninstall += ['IfFileExists "$INSTDIR\\' + relative + '" 0 skip_' + str(number),
                          'ClearErrors', 'Delete "$INSTDIR\\' + relative + '"',
                          'IfErrors uninstall_failed', 'skip_' + str(number) + ':']
            for directory in PurePosixPath(name).parents:
                if str(directory) != '.':
                    directories.add(str(directory).replace('/', '\\'))
        uninstall += ['RMDir "$INSTDIR\\' + directory + '"'
                      for directory in sorted(directories, key=lambda s: s.count('\\'), reverse=True)]
        (root / 'install-files.nsh').write_text('\n'.join(install), encoding='utf-8-sig')
        (root / 'uninstall-files.nsh').write_text('\n'.join(uninstall), encoding='utf-8-sig')
        script = Path(__file__).resolve().parent.parent / 'installer/setupscope.nsi'
        subprocess.run([str(compiler), '/V2', '/DPAYLOAD=' + str(root),
                        '/DOUTPUT=' + str(output), str(script)], check=True)
    print(hashlib.sha256(output.read_bytes()).hexdigest() + '  ' + output.name)


if __name__ == '__main__':
    main()
