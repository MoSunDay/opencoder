use super::wal;
use anyhow::{bail, ensure, Context, Result};
use std::{
    io::{Read, Write},
    path::Path,
};

fn bounded_read(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "resource database file size limit exceeded"
    );
    Ok(bytes)
}

/// Query only an isolated copy; the database connection never opens the source files.
/// WAL frames are validated and copied; the derived WAL index is rebuilt privately.
pub(super) async fn strings(source: &Path, bytes: &[u8]) -> Result<Vec<String>> {
    let source_wal = wal::companion(source, "-wal");
    let wal_bytes = match std::fs::symlink_metadata(&source_wal) {
        Ok(meta) => {
            ensure!(
                meta.file_type().is_file() && meta.len() <= 4 * 1024 * 1024,
                "unverified resource WAL file"
            );
            let mut data = Vec::new();
            std::fs::File::open(&source_wal)?
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut data)?;
            ensure!(
                data.len() <= 4 * 1024 * 1024,
                "resource WAL size limit exceeded"
            );
            wal::validate(&data, bytes)?;
            Some(data)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    ensure!(
        bounded_read(source)? == bytes,
        "resource database changed during inspection"
    );
    match &wal_bytes {
        Some(data) => ensure!(
            bounded_read(&source_wal)? == *data,
            "resource WAL changed during inspection"
        ),
        None => ensure!(
            !source_wal.try_exists()?,
            "resource WAL appeared during inspection"
        ),
    }
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("resource.db");
    std::fs::File::create(&path)?.write_all(bytes)?;
    if let Some(data) = &wal_bytes {
        std::fs::File::create(wal::companion(&path, "-wal"))?.write_all(data)?;
    }
    // The application database engine reads only UTF-8 databases, so files
    // with any other text encoding fail verification and keep deletions
    // blocked. This reader stays inside the existing blocking resource scan;
    // connections only open the copy, never a live database.
    let database = turso::Builder::new_local(path.to_str().context("resource database copy path")?)
        .read_only(true)
        .build()
        .await
        .context("unreadable resource database")?;
    let connection = database.connect()?;
    let mut check = connection.query("PRAGMA quick_check", ()).await?;
    let row = check
        .next()
        .await?
        .ok_or_else(|| anyhow::anyhow!("empty database validation"))?;
    ensure!(
        row.get::<String>(0)? == "ok" && check.next().await?.is_none(),
        "invalid resource database"
    );
    let mut schema = connection
        .query("SELECT name,sql,type FROM sqlite_schema", ())
        .await?;
    let mut tables = Vec::new();
    let mut found = Vec::new();
    while let Some(row) = schema.next().await? {
        let name = row.get::<String>(0)?;
        let definition = row.get::<Option<String>>(1)?.unwrap_or_default();
        ensure!(
            !definition
                .to_ascii_uppercase()
                .contains("CREATE VIRTUAL TABLE"),
            "unverified virtual resource table"
        );
        found.push(name.clone());
        found.push(definition);
        if row.get::<String>(2)? == "table" {
            tables.push(name);
        }
    }
    let mut total_bytes = found.iter().map(String::len).sum::<usize>();
    let mut total_rows = 0;
    for name in tables {
        let query = format!("SELECT * FROM \"{}\"", name.replace('"', "\"\""));
        let mut rows = connection.query(&query, ()).await?;
        while let Some(row) = rows.next().await? {
            total_rows += 1;
            ensure!(
                total_rows <= 100_000,
                "resource database row limit exceeded"
            );
            for index in 0..row.column_count() {
                let text = match row.get_value(index)? {
                    turso::Value::Text(text) => Some(text),
                    turso::Value::Blob(bytes) => Some(String::from_utf8(bytes).map_err(|_| {
                        anyhow::anyhow!("unverified binary resource database value")
                    })?),
                    turso::Value::Null | turso::Value::Integer(_) | turso::Value::Real(_) => None,
                };
                if let Some(text) = text {
                    if text.contains('\0') {
                        bail!("unverified encoded resource database value");
                    }
                    total_bytes += text.len();
                    ensure!(
                        total_bytes <= 4 * 1024 * 1024,
                        "resource database text limit exceeded"
                    );
                    found.push(text);
                }
            }
        }
    }
    Ok(found)
}
