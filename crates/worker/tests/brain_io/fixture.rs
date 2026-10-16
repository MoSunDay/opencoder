use crate::support::{stage_binary, Fleet};
use serde_json::{json, Value};

pub(super) const READER: &str = r#"
import hashlib,json,os,pathlib
context=json.loads(pathlib.Path(os.environ['OPENCODER_STEP_CONTEXT']).read_text())
payload=context['inputs']['payload']
assert payload['flag'] is False and payload['zero']==0 and payload['empty']==[]
try:
    pathlib.Path(os.environ['OPENCODER_STEP_CONTEXT']).write_text('changed')
except OSError:
    readonly=True
else:
    raise AssertionError('step context must be read-only')
content=(payload['seed']+'\n').encode()
pathlib.Path('payload.txt').write_bytes(content)
pathlib.Path('artifacts.json').write_text(json.dumps({'files':[{'path':'payload.txt','bytes':len(content),'sha256':hashlib.sha256(content).hexdigest()}]}))
pathlib.Path('output.json').write_text(json.dumps({'marker':payload['seed'],'revision':payload['revision'],'readonly':readonly,
    'namespace':os.readlink('/proc/self/ns/mnt'),'instance':context.get('instance'),'origin':context['inputs']['origin']}))
"#;

pub async fn prepare(fleet: &Fleet) {
    let program = format!("#include <unistd.h>\nint main(void) {{ execl(\"/usr/bin/python3\",\"python3\",\"-c\",{},(char*)0); return 127; }}", serde_json::to_string(READER).unwrap());
    for index in 0..fleet.nodes.len() {
        stage_binary(
            &fleet.root().join(format!("n{index}/node")),
            "io-reader",
            &program,
        );
    }
    let saved = fleet.call("POST", "/api/dag/defs", json!({"spec":{"name":"io-dag","steps":[
        {"name":"inspect","kind":{"type":"binary","resource":"io-reader"}},
        {"name":"expand","depends_on":["inspect"],"kind":{"type":"dynamic","source":{"type":"input","pointer":"/items"},"template":{"type":"binary","resource":"io-reader"}}},
        {"name":"describe","depends_on":["inspect"],"kind":{"type":"agent","prompt":"Describe the frozen IO payload"}}
    ]}})).await;
    assert_eq!(saved.status, 200, "{saved:?}");
    for (id, kind, target, definition, inputs, outputs) in [
        (
            "io-producer",
            "agent",
            "act",
            json!({}),
            vec!["seed"],
            vec!["seed", "revision", "items"],
        ),
        (
            "io-dag",
            "dag",
            "io-dag",
            saved.body,
            vec!["payload", "items", "origin"],
            vec!["inspect", "expand", "describe"],
        ),
        (
            "io-consumer",
            "operator",
            "act",
            json!({}),
            vec!["output", "files", "origin", "seed"],
            vec!["passed", "revision", "origin"],
        ),
    ] {
        fleet.state.fleet.put_definition("brain_capability",id,&json!({"id":id,"kind":kind,"target":target,
            "version":"fixture","summary":id,"input_desc":"exact bound values","output_desc":"verified structured IO",
            "required_inputs":inputs,"required_outputs":outputs,"definition":definition})).await.unwrap();
    }
}

pub fn plan() -> Value {
    json!({"schema_version":7,"title":"IO flow","objective":"transfer exact values and verified artifacts",
    "max_rounds":3,"layers":[
        {"layer_id":"produce","title":"Produce","task":"produce","objective":"produce exact values","success_criteria":"values exist"},
        {"layer_id":"dag","title":"DAG","task":"inspect frozen inputs","objective":"create indexed artifacts","success_criteria":"all steps see exact inputs"},
        {"layer_id":"consume","title":"Consume","task":"verify transferred output and files","objective":"validate the source","success_criteria":"passed is true"}
    ],"nodes":[
        {"node_id":"produce","layer_id":"produce","title":"Produce","objective":"produce exact values","capability_id":"io-producer"},
        {"node_id":"dag","layer_id":"dag","title":"DAG","objective":"inspect frozen inputs","capability_id":"io-dag"},
        {"node_id":"consume","layer_id":"consume","title":"Consume","objective":"verify output and files","capability_id":"io-consumer"}
    ]})
}

pub fn wrapper() -> Value {
    json!({"schema_version":7,"title":"IO wrapper","objective":"use the nested artifact manifest",
    "layers":[
        {"layer_id":"nested","title":"Nested","task":"run frozen plan","objective":"produce verified files","success_criteria":"child completed"},
        {"layer_id":"consume","title":"Consume","task":"read nested manifest","objective":"verify files","success_criteria":"passed is true"}
    ],"nodes":[
        {"node_id":"nested","layer_id":"nested","title":"Nested","objective":"run the frozen child plan","capability_id":"plan-io-child@1"},
        {"node_id":"consume","layer_id":"consume","title":"Consume","objective":"verify child output and files","capability_id":"io-consumer"}
    ]})
}
