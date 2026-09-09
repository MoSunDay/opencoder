"""Use the release installer's verification without installing any binaries."""
from pathlib import Path
import importlib.util


def verify_platform(bundle):
    source = Path(__file__).resolve().parents[3] / 'platform/install_bundle.py'
    spec = importlib.util.spec_from_file_location('acceptance_platform_installer', source)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    bundle = Path(bundle).resolve()
    manifest = module.verify_bundle(bundle)
    if set(manifest['files']) != {f'bin/{name}' for name in module.NAMES}:
        raise ValueError('Linux acceptance requires all four platform binaries')
    return manifest, {name: str(bundle / 'bin' / name) for name in module.NAMES}
