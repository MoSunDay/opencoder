//! Diamond workflow end-to-end across the full platform stack: a binary `a`
//! fans out into binary `b`/`c` that each really READ `a`'s context.json and
//! transform its output, converging in an agent `d` that summarizes all
//! upstream outputs. Real control app + WS node + node binary runtime + real
//! agent session; only the LLM is scripted (zero real models, no network).

use super::*;

/// The DAG agent step captures its transcript from TextDelta frames, so a
/// scripted answer needs the delta plus the terminal Completed event.
fn completed(text: String) -> Vec<LlmEvent> {
    vec![
        LlmEvent::TextDelta(text.clone()),
        LlmEvent::Completed {
            text,
            tool_calls: vec![],
            usage: None,
        },
    ]
}

fn echo_source() -> String {
    r#"#include <stdio.h>
int main(void) { FILE *output=fopen("output.json","w"); if(!output)return 2; fputs("{\"value\":1}",output); fclose(output); puts("{\"value\":1}"); return 0; }"#.into()
}

fn delta_source(delta: u32) -> String {
    r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(void) { char buffer[65536]; FILE *context=fopen(getenv("OPENCODER_STEP_CONTEXT"),"r"); if(!context)return 2;
size_t count=fread(buffer,1,sizeof(buffer)-1,context); fclose(context); buffer[count]=0;
char *value=strstr(buffer,"\"value\""); if(!value)return 3; value=strchr(value,':'); if(!value)return 4;
int result=atoi(value+1)+@DELTA@; FILE *output=fopen("output.json","w"); if(!output)return 5;
fprintf(output,"{\"value\":%d}",result); fclose(output); printf("{\"value\":%d}\n",result); return 0; }"#.replace("@DELTA@", &delta.to_string())
}

#[tokio::test]
async fn diamond_workflow_binary_steps_feed_the_agent_step() {
    let _config = support::isolated_config();
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    let data_dir = fleet.root().join("n0/node");
    support::stage_binary(&data_dir, "echo", &echo_source());
    support::stage_binary(&data_dir, "plus1", &delta_source(1));
    support::stage_binary(&data_dir, "plus3", &delta_source(3));
    // The agent step answers with prose around a ```json fence so the run
    // loop recovers structured output via `extract_output_json_from`.
    client.queue_script(completed(
        "summary ready\n```json\n{\"total\":6,\"from\":{\"b\":2,\"c\":4}}\n```\n".to_string(),
    ));
    let saved = fleet
        .call(
            "POST",
            "/api/dag/defs",
            json!({"spec":{"name":"diamond","steps":[
                {"name":"a","kind":{"type":"binary","resource":"echo"}},
                {"name":"b","depends_on":["a"],"kind":{"type":"binary","resource":"plus1"}},
                {"name":"c","depends_on":["a"],"kind":{"type":"binary","resource":"plus3"}},
                {"name":"d","depends_on":["b","c"],
                 "kind":{"type":"agent","prompt":"汇总 b 与 c 的结果"}}
            ]}}),
        )
        .await;
    assert_eq!(saved.status, 200, "{saved:?}");
    let dispatched = fleet
        .call(
            "POST",
            "/api/dag/defs/diamond/dispatch",
            json!({"id":"dag-diamond-1"}),
        )
        .await;
    assert_eq!(dispatched.status, 202, "{dispatched:?}");
    let detail = settled(&fleet.nodes[0], "dag-diamond-1").await;
    assert_eq!(detail["execution"]["status"], "done", "{detail}");

    // 1. Step artifacts: the binary steps really computed a → b/c and the
    //    agent step recovered the fenced summary as its output.json.
    let run = support::dag_run(&data_dir, "dag-diamond-1");
    let output = |step: &str| -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(run.join(step).join("output.json"))
                .unwrap_or_else(|e| panic!("{step}/output.json missing: {e}")),
        )
        .unwrap()
    };
    assert_eq!(output("a"), json!({"value":1}));
    assert_eq!(output("b"), json!({"value":2}));
    assert_eq!(output("c"), json!({"value":4}));
    assert_eq!(output("d"), json!({"total":6,"from":{"b":2,"c":4}}));

    // 2. The binary steps really READ the upstream context: b's context.json
    //    carries a's structured output plus its success flag.
    let b_ctx: Value =
        serde_json::from_str(&std::fs::read_to_string(run.join("b/meta/context.json")).unwrap())
            .unwrap();
    assert_eq!(b_ctx["steps"]["a"]["json"], json!({"value":1}));
    assert_eq!(b_ctx["steps"]["a"]["ok"], json!(true));

    // 3. The agent step really received a/b/c outputs: its user prompt
    //    embeds the pretty upstream context (`a` is a TRANSITIVE upstream
    //    of `d`, so all three values must appear in the same prompt).
    let requests = client.requests();
    let agent_prompt = requests
        .iter()
        .flat_map(|request| request.messages.iter())
        .map(|message| message.text())
        .find(|content| content.contains("汇总 b 与 c 的结果"))
        .expect("agent step prompt request missing");
    for fragment in ["\"value\": 1", "\"value\": 2", "\"value\": 4"] {
        assert!(
            agent_prompt.contains(fragment),
            "prompt misses upstream {fragment}: {agent_prompt}"
        );
    }
    fleet.shutdown().await;
}
