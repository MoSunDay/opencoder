use opencoder_core::brain::*;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum Resolution {
    Value(Value),
    Waiting(String),
    Unavailable(String),
}

pub fn resolve(run: &BrainRun, binding: &Binding, item: Option<&Value>) -> Resolution {
    match binding {
        Binding::Literal { value } => Resolution::Value(value.clone()),
        Binding::Input { name, path } => match run.request.inputs.get(name) {
            Some(value) => pointer(value, path),
            None => Resolution::Waiting(format!("input:{name}")),
        },
        Binding::Item { path } => item
            .map(|v| pointer(v, path))
            .unwrap_or_else(|| Resolution::Unavailable("item is unavailable".into())),
        Binding::Output {
            step,
            path,
            collect,
        } => {
            let Some(ids) = run.expansions.get(step) else {
                return Resolution::Waiting(format!("expansion:{step}"));
            };
            let mut values = Vec::new();
            for id in ids {
                let instance = &run.instances[id];
                if !instance.status.terminal() {
                    return Resolution::Waiting(format!("step:{id}"));
                }
                if instance.status == StepStatus::Skipped && *collect {
                    continue;
                }
                let Some(output) = instance
                    .output
                    .as_ref()
                    .filter(|_| instance.status == StepStatus::Succeeded)
                else {
                    return Resolution::Unavailable(format!(
                        "{id}: {:?}: {}",
                        instance.status, instance.reason
                    ));
                };
                match pointer(&output.value, path) {
                    Resolution::Value(v) => values.push(v),
                    other => return other,
                }
            }
            if *collect {
                Resolution::Value(Value::Array(values))
            } else if values.len() == 1 {
                Resolution::Value(values.remove(0))
            } else {
                Resolution::Unavailable(format!("{step} did not produce a scalar output"))
            }
        }
    }
}

fn pointer(value: &Value, path: &str) -> Resolution {
    value
        .pointer(path)
        .cloned()
        .map(Resolution::Value)
        .unwrap_or_else(|| Resolution::Unavailable(format!("output path {path} is absent")))
}
