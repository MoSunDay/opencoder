use crate::{database::Store, error::AppError};
use std::{collections::HashMap, path::Path};
use turso::Connection;

#[cfg(test)]
mod tests;

/// Existing databases never migrate as a side effect of ordinary startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaPolicy {
    InitializeEmpty,
    Validate,
    Maintenance,
}

// The release controller embeds this same contract; its tests prevent drift.
pub const CONTRACT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/platform/rolling/native/ontology_schema.sql"
));

fn normalize(sql: &str) -> String {
    let mut quoted = false;
    let mut normalized = String::new();
    for c in sql.chars() {
        if c == '\'' {
            quoted = !quoted;
        }
        if quoted || c == '\'' {
            normalized.push(c);
        } else if !c.is_ascii_whitespace() && c != ';' {
            normalized.push(c.to_ascii_lowercase());
        }
    }
    for prefix in ["createtable", "createindex", "createuniqueindex"] {
        normalized = normalized.replacen(&format!("{prefix}ifnotexists"), prefix, 1);
    }
    normalized
}

fn statements() -> impl Iterator<Item = (&'static str, &'static str)> {
    CONTRACT
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|sql| {
            let prefix = sql.split('(').next().unwrap();
            let mut words = prefix.split_whitespace();
            words.next();
            if words.next() == Some("UNIQUE") {
                words.next();
            }
            (words.next().unwrap(), sql)
        })
}

async fn objects(connection: &Connection) -> Result<HashMap<String, String>, AppError> {
    let mut rows = connection
        .query(
            "SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'",
            (),
        )
        .await?;
    let mut found = HashMap::new();
    while let Some(row) = rows.next().await? {
        found.insert(row.get(0)?, row.get(1)?);
    }
    Ok(found)
}

/// Validate every existing object before creating any missing object.
async fn inspect(
    connection: &Connection,
    files: &Path,
) -> Result<(bool, Vec<&'static str>), AppError> {
    let found = objects(connection).await?;
    let empty = found.is_empty();
    let mut missing = Vec::new();
    for (name, sql) in statements() {
        match found.get(name) {
            Some(actual) if normalize(actual) == normalize(sql) => {},
            Some(_) => return Err(AppError::config(format!("Ontology schema conflicts with {name}; maintenance cannot replace existing objects"))),
            None => missing.push(sql),
        }
    }
    if !empty {
        if !found.contains_key("ontology_schema_version") {
            return Err(AppError::config(
                "Ontology database lacks its files binding",
            ));
        }
        let mut rows = connection
            .query("SELECT version,files_root FROM ontology_schema_version", ())
            .await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| AppError::config("Ontology database lacks its files binding"))?;
        if row.get::<i64>(0)? != 1
            || row.get::<String>(1)? != files.to_string_lossy()
            || rows.next().await?.is_some()
        {
            return Err(AppError::config("ontology files directory differs from the database binding; retain the original directory and restore the database and text files together"));
        }
    }
    Ok((empty, missing))
}

async fn has_debug(connection: &Connection) -> Result<bool, AppError> {
    Ok(connection
        .query("SELECT 1 FROM environments WHERE env_key='debug'", ())
        .await?
        .next()
        .await?
        .is_some())
}

pub(super) async fn ready(
    connection: &Connection,
    files: &Path,
    policy: SchemaPolicy,
) -> Result<bool, AppError> {
    let (empty, missing) = inspect(connection, files).await?;
    let complete = missing.is_empty() && has_debug(connection).await?;
    if complete {
        return Ok(true);
    }
    if policy != SchemaPolicy::Maintenance && !(empty && policy == SchemaPolicy::InitializeEmpty) {
        return Err(AppError::config("Ontology schema requires --maintenance before starting a candidate; normal startup cannot change shared storage"));
    }
    Ok(false)
}

pub(super) async fn bootstrap(
    connection: &Connection,
    files: &Path,
    policy: SchemaPolicy,
) -> Result<(), AppError> {
    connection.execute("BEGIN IMMEDIATE", ()).await?;
    let result = apply(connection, files, policy).await;
    connection
        .execute(if result.is_ok() { "COMMIT" } else { "ROLLBACK" }, ())
        .await?;
    result
}

async fn apply(
    connection: &Connection,
    files: &Path,
    policy: SchemaPolicy,
) -> Result<(), AppError> {
    if ready(connection, files, policy).await? {
        return Ok(());
    }
    let (empty, missing) = inspect(connection, files).await?;
    for sql in missing {
        connection.execute(sql, ()).await?;
    }
    if empty {
        connection
            .execute(
                "INSERT INTO ontology_schema_version VALUES(1,?1)",
                [files.to_string_lossy().to_string()],
            )
            .await?;
    }
    Store {
        connection: connection.clone(),
        include_pending: false,
    }
    .initialize_debug()
    .await?;
    ready(connection, files, SchemaPolicy::Validate).await?;
    Ok(())
}
