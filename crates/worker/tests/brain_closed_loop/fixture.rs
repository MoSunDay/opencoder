use crate::support::{stage_binary, Fleet};
use serde_json::json;

const CODE: &str = r#"
import hashlib, json, pathlib, sys
version = int(sys.argv[1])
source = 'def answer():\n    return %d\n' % version
pathlib.Path('answer.py').write_text(source)
revision = hashlib.sha256(source.encode()).hexdigest()
result = {'summary':'Code prepared', 'revision':revision, 'source':source, 'args':[source]}
pathlib.Path('output.json').write_text(json.dumps(result))
pathlib.Path('artifacts.json').write_text(json.dumps({'files':[{'path':'answer.py','bytes':len(source.encode()),'sha256':revision}]}))
"#;

const TEST: &str = r#"
import hashlib, json, pathlib, subprocess, sys
source = sys.argv[1]
pathlib.Path('answer.py').write_text(source)
check = subprocess.run(['/usr/bin/python3','-c','from answer import answer; assert answer() == 2'], capture_output=True, text=True)
result = {'summary':'Tests executed', 'passed':check.returncode == 0,
          'failures':[] if check.returncode == 0 else [check.stderr],
          'revision':hashlib.sha256(source.encode()).hexdigest(), 'args':['2']}
pathlib.Path('output.json').write_text(json.dumps(result))
"#;

pub async fn prepare(fleet: &Fleet) {
    for (name, step, script) in [("coding", "code", CODE), ("testing", "verify", TEST)] {
        let program = format!(
            "#include <unistd.h>\nint main(int argc,char **argv) {{ execl(\"/usr/bin/python3\",\"python3\",\"-c\",{},argc>1?argv[1]:\"\",(char*)0); return 127; }}",
            serde_json::to_string(script).unwrap()
        );
        stage_binary(&fleet.root().join("n0/node"), name, &program);
        let saved = fleet.call("POST", "/api/dag/defs", json!({"spec":{
            "name":name,"steps":[{"name":step,"timeout_secs":30,"kind":{"type":"binary","resource":name}}]
        }})).await;
        assert_eq!(saved.status, 200, "{saved:?}");
        fleet.state.fleet.put_definition("brain_capability", name, &json!({
            "id":name,"kind":"dag","target":name,"version":"fixture",
            "summary":name,"input_desc":"args supplies exact task values",
            "output_desc":"structured evidence with revision and args for the next capability",
            "required_inputs":["args"],"required_outputs":[step],"definition":saved.body
        })).await.unwrap();
    }
}

pub fn plan() -> serde_json::Value {
    json!({"schema_version":7,"title":"Verified code change","objective":"answer() must return 2",
    "max_rounds":5,"layers":[
        {"layer_id":"coding","title":"Code","task":"implement or repair","objective":"prepare code","success_criteria":"exact source and revision are available"},
        {"layer_id":"testing","title":"Test","task":"test the supplied source","objective":"verify the change","success_criteria":"passed is true for the current revision"}
    ],"nodes":[
        {"node_id":"code","layer_id":"coding","title":"Code","objective":"prepare source","capability_id":"coding"},
        {"node_id":"test","layer_id":"testing","title":"Test","objective":"verify source","capability_id":"testing"}
    ]})
}
