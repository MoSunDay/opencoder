"""Copy existing CLI login state into the attempt's private account directories."""
from pathlib import Path
import shutil


def identity_paths(home, private_home, environment):
    data = Path(environment.get('XDG_DATA_HOME') or home / '.local/share')
    config = Path(environment.get('XDG_CONFIG_HOME') or home / '.config')
    return [(home / name, private_home / name)
            for name in ['.fornax-cli', '.lark-cli', '.byte_cli']] + [
        (data / 'bytedcli/data', private_home / '.local/share/bytedcli/data'),
        (config / 'bytedcli', private_home / '.config/bytedcli')]


def snapshot_identity(home, private_home, environment):
    for source, target in identity_paths(home, private_home, environment):
        if not source.exists():
            continue
        # Refuse reused destinations; never overwrite an existing login state.
        shutil.copytree(source, target, symlinks=False)
        target.chmod(0o700)
        for path in target.rglob('*'):
            path.chmod(0o700 if path.is_dir() else 0o600)
