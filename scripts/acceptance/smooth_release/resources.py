"""Private read-only NFS exports and mounts, retained across business switches."""
import subprocess


class Resources:
    def __init__(self, root, allocate_port):
        self.roots = {}
        self.mounts = {}
        self.ports = {}
        self.mounted = []
        for section in ['agent','dag']:
            source = root / 'exports' / section
            mount = root / 'mounts' / section
            source.mkdir(parents=True)
            mount.mkdir(parents=True)
            (source / 'release-evidence.txt').write_text('resource-service-kept-running')
            self.roots[section], self.mounts[section] = source, mount
            self.ports[section] = allocate_port()

    def server_config(self):
        return {section:{key:str(self.roots[section]),'nfs':{
            'enabled':True,'host':'127.0.0.1','port':self.ports[section],'read_only':True}}
            for section,key in [('agent','agents_dir'),('dag','wasm_dir')]}

    def client_config(self):
        return {section:{key:str(self.mounts[section])} for section,key in [('agent','agents_dir'),('dag','wasm_dir')]}

    def mount(self):
        for section in ['agent','dag']:
            port = self.ports[section]
            subprocess.run(['mount','-t','nfs','-o',
                f'ro,vers=3,tcp,port={port},mountport={port},nolock,soft,retrans=1,timeo=50,actimeo=0,lookupcache=none',
                '127.0.0.1:/',str(self.mounts[section])],check=True,timeout=30)
            self.mounted.append(self.mounts[section])
        self.check()

    def check(self):
        for mount in self.mounted:
            options = subprocess.check_output(['findmnt','-n','-o','OPTIONS','--mountpoint',str(mount)],text=True).strip().split(',')
            assert 'ro' in options and 'rw' not in options
            assert (mount / 'release-evidence.txt').read_text() == 'resource-service-kept-running'

    def close(self):
        for mount in reversed(self.mounted):
            subprocess.run(['umount',str(mount)],check=True,timeout=30)
