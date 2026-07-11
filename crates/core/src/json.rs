use serde_json::{Map, Value};

pub fn empty_object() -> Value {
    Value::Object(Map::new())
}

pub fn object_schema(properties: Value, required: &[&str]) -> Value {
    let mut m = Map::new();
    m.insert("type".into(), Value::String("object".into()));
    m.insert("properties".into(), properties);
    m.insert("additionalProperties".into(), Value::Bool(false));
    m.insert(
        "required".into(),
        Value::Array(
            required
                .iter()
                .map(|s| Value::String((*s).to_string()))
                .collect(),
        ),
    );
    Value::Object(m)
}

pub fn prop_str(desc: &str) -> Value {
    serde_json::json!({ "type": "string", "description": desc })
}

pub fn prop_array_str(desc: &str) -> Value {
    serde_json::json!({ "type": "array", "items": { "type": "string" }, "description": desc })
}

pub fn opt(value: Value) -> Value {
    let mut o = match value {
        Value::Object(m) => m,
        other => {
            let mut m = Map::new();
            m.insert("base".into(), other);
            return Value::Object(m);
        }
    };
    o.insert(
        "description".into(),
        o.get("description").cloned().unwrap_or(Value::Null),
    );
    Value::Object(o)
}
