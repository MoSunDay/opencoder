use anyhow::{bail, ensure, Result};
use opencoder_core::brain::{DataSchema, DataType};
use serde_json::Value;

pub fn accepts(schema: &DataSchema, value: &Value) -> Result<()> {
    if value.is_null() && schema.nullable {
        return Ok(());
    }
    let valid = match schema.kind {
        DataType::String => value.is_string(),
        DataType::Number => value.is_number(),
        DataType::Integer => value.is_i64() || value.is_u64(),
        DataType::Boolean => value.is_boolean(),
        DataType::Object => value.is_object(),
        DataType::Array => value.is_array(),
        DataType::Null => value.is_null(),
    };
    ensure!(valid, "expected {:?}, received {value}", schema.kind);
    ensure!(
        schema.values.is_empty() || schema.values.contains(value),
        "value is outside declared enum"
    );
    if let Some(object) = value.as_object() {
        for key in &schema.required {
            ensure!(object.contains_key(key), "missing required field {key}");
        }
        for (key, field) in &schema.properties {
            if let Some(value) = object.get(key) {
                accepts(field, value).map_err(|e| anyhow::anyhow!("{key}: {e}"))?;
            }
        }
    }
    if let Some(array) = value.as_array() {
        let item = schema
            .items
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("array schema requires items"))?;
        for (i, value) in array.iter().enumerate() {
            accepts(item, value).map_err(|e| anyhow::anyhow!("[{i}]: {e}"))?;
        }
    }
    Ok(())
}

pub fn compatible(source: &DataSchema, destination: &DataSchema) -> bool {
    if source.kind != destination.kind
        && !(source.kind == DataType::Integer && destination.kind == DataType::Number)
    {
        return false;
    }
    if source.nullable && !destination.nullable
        || destination.semantic.is_some() && source.semantic != destination.semantic
    {
        return false;
    }
    if !destination.values.is_empty()
        && (source.values.is_empty()
            || source
                .values
                .iter()
                .any(|v| !destination.values.contains(v)))
    {
        return false;
    }
    match destination.kind {
        DataType::Object => {
            destination.required.iter().all(|key| {
                source.required.contains(key)
                    && source
                        .properties
                        .get(key)
                        .zip(destination.properties.get(key))
                        .is_some_and(|(s, d)| compatible(s, d))
            }) && destination
                .properties
                .iter()
                .all(|(key, d)| source.properties.get(key).is_none_or(|s| compatible(s, d)))
        }
        DataType::Array => source
            .items
            .as_ref()
            .zip(destination.items.as_ref())
            .is_some_and(|(s, d)| compatible(s, d)),
        _ => true,
    }
}

pub fn projected<'a>(schema: &'a DataSchema, pointer: &str) -> Result<&'a DataSchema> {
    if pointer.is_empty() {
        return Ok(schema);
    }
    ensure!(
        pointer.starts_with('/'),
        "path must be a JSON pointer: {pointer}"
    );
    let mut current = schema;
    for part in pointer[1..].split('/') {
        let key = part.replace("~1", "/").replace("~0", "~");
        current = match current.kind {
            DataType::Object => current
                .properties
                .get(&key)
                .ok_or_else(|| anyhow::anyhow!("unknown field {key}"))?,
            DataType::Array if key.parse::<usize>().is_ok() => current
                .items
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("missing items"))?,
            _ => bail!("cannot project {key} from {:?}", current.kind),
        };
    }
    Ok(current)
}

pub(crate) fn validate_schema(schema: &DataSchema, depth: usize) -> Result<()> {
    ensure!(depth < 32, "schema nesting exceeds 32");
    ensure!(
        schema.kind == DataType::Array || schema.items.is_none(),
        "items only belongs to arrays"
    );
    ensure!(
        schema.kind == DataType::Object
            || (schema.properties.is_empty() && schema.required.is_empty()),
        "properties only belong to objects"
    );
    for key in &schema.required {
        ensure!(
            schema.properties.contains_key(key),
            "required property {key} is undefined"
        );
    }
    if schema.kind == DataType::Array {
        ensure!(schema.items.is_some(), "array requires items");
    }
    for item in schema
        .properties
        .values()
        .chain(schema.items.iter().map(AsRef::as_ref))
    {
        validate_schema(item, depth + 1)?;
    }
    for value in &schema.values {
        accepts(schema, value)?;
    }
    Ok(())
}
