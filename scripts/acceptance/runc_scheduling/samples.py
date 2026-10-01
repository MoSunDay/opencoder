import json
import hashlib
import math
from pathlib import Path
import subprocess
import time
from runtime import inventory, wait, write


SOURCE = r'''
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int main(int count, char **arguments) {
    const char *mode = count > 1 ? arguments[1] : "write";
    FILE *file;
    char namespace_id[256] = {0};
    readlink("/proc/self/ns/pid", namespace_id, sizeof(namespace_id)-1);
    printf("namespace=%s\n", namespace_id); fflush(stdout);
    if (!strcmp(mode,"flood")) { for (int index=0;index<9437184;index++) putchar('x'); fflush(stdout); return 9; }
    if (!strcmp(mode,"spin")) { for (;;) pause(); }
    if (!strcmp(mode,"fail")) { puts("visible failure"); return 9; }
    if (!strcmp(mode,"blob")) {
        char buffer[65536] = {0};
        file=fopen("report.bin","w"); if(!file) return 20;
        for(int block=0;block<256;block++) if(fwrite(buffer,1,sizeof(buffer),file)!=sizeof(buffer)) return 21;
        fclose(file);
        file=fopen("artifacts.json","w"); if(!file) return 22;
        fputs("{\"files\":[{\"path\":\"report.bin\",\"bytes\":16777216,\"sha256\":\"BLOB_DIGEST\"}]}",file); fclose(file);
        file=fopen("not-declared.txt","w"); fputs("do not archive",file); fclose(file);
    }
    if (!strcmp(mode,"bad-artifact")) {
        file=fopen("artifacts.json","w"); if(!file) return 23;
        fputs("{\"files\":[{\"path\":\"../escape\",\"bytes\":0,\"sha256\":\"BLOB_DIGEST\"}]}",file); fclose(file);
    }
    if (!strcmp(mode,"dynamic")) {
        char directory[512]; if(!getcwd(directory,sizeof(directory))) return 24;
        printf("cwd=%s\n",directory);
        for(int index=2;index<count;index++) printf("argument=%s\n",arguments[index]);
    }
    if (!strcmp(mode,"recover")) {
        file=fopen("waiting","w"); if(!file) return 25; fclose(file);
        while(access("release",F_OK)) usleep(50000);
    }
    if (!strcmp(mode,"consume") || !strcmp(mode,"recover")) {
        file=fopen("../first/shared.txt","r"); if(!file) return 11;
        fclose(file);
    } else {
        int attempts=0;
        file=fopen("attempts","r"); if(file) { fscanf(file,"%d",&attempts); fclose(file); }
        file=fopen("attempts","w"); if(!file) return 26; fprintf(file,"%d",attempts+1); fclose(file);
        file=fopen("shared.txt","w"); if(!file) return 12;
        fputs("shared from first",file); fclose(file);
    }
    file=fopen("seed.txt","w"); if(!file) { perror("COW seed"); return 13; }
    fputs("node COW update",file); fclose(file);
    file=fopen(getenv("OPENCODER_STEP_CONTEXT"),"w"); if(file) return 14;
    file=fopen("/etc/hosts","w"); if(file) return 15;
    file=fopen("output.json","w"); if(!file) return 16;
    fputs("{\"ok\":true}",file); fclose(file);
    puts("portable native runc"); return 0;
}
'''
SOURCE = SOURCE.replace('BLOB_DIGEST', hashlib.sha256(bytes(16777216)).hexdigest())


def submit(runtime, identifier, resource, node, mode='write', timeout=30, paired=False):
    steps = [{'name': 'first', 'timeout_secs': timeout,
              'kind': {'type': 'binary', 'resource': resource, 'args': [mode]}}]
    if paired:
        steps.append({'name': 'second', 'depends_on': ['first'], 'timeout_secs': timeout,
            'kind': {'type': 'binary', 'resource': resource, 'args': ['consume']}})
    return runtime.api('/api/executions', 'POST', {'id': identifier, 'kind': 'dag', 'node_id': node,
        'input': {'definition': {'name': 'native-acceptance', 'steps': steps}}})


def finished(runtime, identifier):
    value = runtime.api('/api/executions/' + identifier)
    return value if value['execution']['status'] in ['done', 'error', 'cancelled'] else None


def clean(root):
    state = subprocess.run(['runc', '--root', str(root / 'runc-state'), 'list', '--format', 'json'],
                           capture_output=True, text=True, check=True)
    assert not json.loads(state.stdout), state.stdout
    assert not any(str(root) in line for line in Path('/proc/self/mountinfo').read_text().splitlines())


def observe(runtime, nodes, seconds):
    started = time.monotonic()
    deadline = started + seconds
    receipts = []
    while time.monotonic() < deadline:
        name = ['node-a', 'node-b'][len(receipts) % 2]
        identifier = 'dag-native-observe-' + str(len(receipts))
        began = time.monotonic()
        submit(runtime, identifier, 'native-acceptance@v1', nodes[name], paired=True)
        value = wait(lambda: finished(runtime, identifier), identifier, seconds=30)
        assert value['execution']['status'] == 'done', value
        root = runtime.run_root(name, identifier)
        meta = json.loads((root / 'first/meta.json').read_text())
        cold = (meta['started_at_ms'] - value['execution']['created_at']) / 1000
        assert cold <= 10, {'id': identifier, 'cold_seconds': cold}
        clean(root)
        assert inventory(runtime.source) == runtime.original
        receipts.append({'id': identifier, 'node': name, 'cold_seconds': cold,
                         'terminal_seconds': time.monotonic() - began})
        write(runtime.root / 'evidence/observation.json', receipts)
        time.sleep(min(10, max(0, deadline - time.monotonic())))
    return {'observation_seconds': time.monotonic() - started, 'observation_samples': len(receipts)}


def exercise(runtime, nodes):
    resource = runtime.publish('native-acceptance', SOURCE)
    timings, receipts = [], []
    for index in range(20):
        name = ['node-a', 'node-b'][index % 2]
        identifier = f'dag-native-sample-{index:02}'
        started = time.monotonic()
        submit(runtime, identifier, resource, nodes[name], paired=True)
        value = wait(lambda: finished(runtime, identifier), identifier)
        assert value['execution']['status'] == 'done', value
        assert value['execution']['node_id'] == nodes[name], value
        root = runtime.run_root(name, identifier)
        outputs = [(root / step / 'output.txt').read_text() for step in ['first', 'second']]
        assert outputs[0].splitlines()[0] == outputs[1].splitlines()[0], outputs
        assert (root / 'upper/first/shared.txt').is_file()
        assert (root / 'upper/first/seed.txt').read_text() == 'node COW update'
        config = json.loads((root / 'bundle/config.json').read_text())
        assert config['root']['readonly']
        first = json.loads((root / 'first/meta.json').read_text())
        cold = (first['started_at_ms'] - value['execution']['created_at']) / 1000
        timings.append(cold)
        clean(root)
        assert inventory(runtime.source) == runtime.original
        receipts.append({'id': identifier, 'node': name, 'cold_seconds': cold,
                         'terminal_seconds': time.monotonic()-started})
    p95 = sorted(timings)[math.ceil(.95*len(timings))-1]
    assert p95 <= 10, {'cold_p95_seconds': p95}
    for mode, timeout in [('fail', 30), ('spin', 1)]:
        identifier = 'dag-native-' + mode
        submit(runtime, identifier, resource, nodes['node-a'], mode, timeout)
        value = wait(lambda: finished(runtime, identifier), identifier)
        assert value['execution']['status'] == 'error', value
        clean(runtime.run_root('node-a', identifier))
    identifier = 'dag-native-cancel'
    submit(runtime, identifier, resource, nodes['node-b'], 'spin')
    def running():
        root = runtime.run_root('node-b', identifier)
        return root if root and (root / 'first/meta/context.json').exists() else None
    root = wait(running, 'Running shared container')
    write(runtime.root / 'evidence/cancel-oci.json', json.loads((root / 'bundle/config.json').read_text()))
    started = time.monotonic()
    runtime.api('/api/executions/' + identifier + '/commands', 'POST', {'action': 'cancel'})
    value = wait(lambda: finished(runtime, identifier), identifier)
    assert value['execution']['status'] == 'cancelled', value
    assert time.monotonic()-started <= 30
    clean(root)
    write(runtime.root / 'evidence/samples.json', receipts)
    from cases import boundaries, recovery
    boundaries(runtime, nodes, resource)
    recovery(runtime, nodes, resource)
    return {'nodes': nodes, 'samples': len(receipts), 'cold_p95_seconds': p95,
            'failed_steps': 4, 'cancelled_runs': 1, 'dynamic_instances': 3,
            'large_artifact_bytes': 16777216, 'worker_restart': True, 'immutable_version_recovery': True}
