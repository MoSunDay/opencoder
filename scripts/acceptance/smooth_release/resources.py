"""Private read-only NFS exports and mounts, retained across platform switches."""
import subprocess


class Resources:
    def __init__(self, root, allocate_port):
        self.roots = {}
        self.mounts = {}
        self.ports = {}
        self.mounted = []
        for section in ['agent','dag','workspace','ontology']:
            source = root / 'exports' / section
            mount = root / 'mounts' / section
            source.mkdir(parents=True)
            mount.mkdir(parents=True)
            (source / 'release-evidence.txt').write_text('resource-service-kept-running')
            self.roots[section], self.mounts[section] = source, mount
            self.ports[section] = allocate_port()

    def server_config(self):
        config = {section:{key:str(self.roots[section]),'nfs':{
            'enabled':True,'host':'127.0.0.1','port':self.ports[section],'read_only':True}}
            for section,key in [('agent','agents_dir'),('dag','binary_dir')]}
        config['dag'].update(workspace_dir=str(self.roots['workspace']), workspace_nfs={
            'enabled':True,'host':'127.0.0.1','port':self.ports['workspace']})
        config['ontology'] = {'files_dir':str(self.roots['ontology']), 'nfs':{'enabled':True,'host':'127.0.0.1','port':self.ports['ontology']}}
        return config

    def client_config(self):
        config = {section:{key:str(self.mounts[section])} for section,key in [('agent','agents_dir'),('dag','binary_dir')]}
        config['dag']['workspace_dir'] = str(self.mounts['workspace'])
        return config

    def mount(self):
        for section in ['agent','dag','workspace','ontology']:
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
