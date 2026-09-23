#!/usr/bin/env python3
"""Install a content-addressed PC issue tool bundle without touching running versions."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil


def install(source, destination):
    files=sorted(source.glob('*.py'))
    digest=hashlib.sha256(b''.join(p.name.encode()+b'\0'+p.read_bytes() for p in files)).hexdigest()
    target=destination/digest
    target.mkdir(parents=True,exist_ok=True,mode=0o755)
    manifest={}
    for path in files:
        content=path.read_bytes(); copied=target/path.name
        if copied.exists() and copied.read_bytes()!=content:
            raise ValueError('Existing tool bundle content differs')
        if not copied.exists():
            shutil.copy2(path,copied)
        manifest[path.name]=hashlib.sha256(content).hexdigest()
    (target/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    temporary=destination/f'.current-{os.getpid()}'
    temporary.symlink_to(target,target_is_directory=True)
    temporary.replace(destination/'current')
    return {'version':digest,'directory':str(target),'helper':str(target/'cli.py')}


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--destination',type=Path,default=Path('/opt/opencoder-pc-issue'))
    args=parser.parse_args()
    print(json.dumps(install(Path(__file__).resolve().parent,args.destination)))
