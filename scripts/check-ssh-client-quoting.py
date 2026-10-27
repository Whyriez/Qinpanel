"""Check API-generated fixture output with real Bash, without invoking clients.

Run ssh_connection_info_test first. Git Bash is used on Windows; no VM or daemon.
Every command/action is a shell function and only captured arguments are emitted.
"""
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess

root = Path(__file__).resolve().parents[1]
data = json.loads((root / 'target/ssh-direct-generated.json').read_text())
bash = (Path(os.environ.get('ProgramFiles', 'C:/Program Files')) / 'Git/bin/bash.exe') if os.name == 'nt' else shutil.which('bash')
assert bash and Path(bash).is_file(), 'Bash is required for the quoting fixture'


def parse_with_mocks(command, mocks):
    # The argument contains only a controlled test fixture. No SSH/SFTP program or cd,
    # exec or command-substitution action is allowed to reach a real binary.
    script = 'id() { printf EXPANSION >&2; };\n' + mocks + '\neval "$1"'
    env = {key: value for key, value in os.environ.items() if key not in ('BASH_ENV', 'ENV')}
    result = subprocess.run([str(bash), '--noprofile', '--norc', '-c', script, 'fixture', command], capture_output=True, env=env, check=False)
    assert result.returncode == 0, result.stderr
    assert not result.stderr, result.stderr
    return result.stdout.decode().rstrip('\0').split('\0')


args = parse_with_mocks(data['ssh_command'], 'ssh() { printf "%s\\0" "$@"; };')
assert args[:4] == ['-p', '2200', '-t', 'p_actor1@2001:db8::2'], args
assert len(args) == 5, args
remote = parse_with_mocks(args[4], 'cd() { printf "%s\\0" "$@"; }; exec() { printf "%s\\0" "$@"; };')
assert remote[0:2] == ['--', data['active_path']], remote
assert len(remote) == 4 and remote[-1] == '-l', remote
assert '$HOME' in remote[1] and '`id`' in remote[1] and '$(id)' in remote[1]
assert parse_with_mocks(data['sftp_command'], 'sftp() { printf "%s\\0" "$@"; };') == ['-P', '2200', 'p_actor1@[2001:db8::2]']
assert shlex.split(data['sftp_cd_command']) == ['cd', data['sftp_path']]
print('Passed 5 quoting checks: local argv, remote literal path, no expansion, SFTP argv, interactive cd')
