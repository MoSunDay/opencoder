use serde_json::Value;

pub fn sanitize_tool_schema(schema: &Value) -> Value {
    let mut obj = match schema {
        Value::Object(m) => m.clone(),
        other => {
            let mut m = serde_json::Map::new();
            m.insert("base".to_string(), other.clone());
            m
        }
    };
    obj.insert("type".to_string(), Value::String("object".into()));
    if !obj.contains_key("properties") {
        obj.insert("properties".to_string(), Value::Object(Default::default()));
    }
    obj.insert("additionalProperties".to_string(), Value::Bool(false));
    if !obj.contains_key("required") {
        obj.insert("required".to_string(), Value::Array(Vec::new()));
    }
    Value::Object(obj)
}
