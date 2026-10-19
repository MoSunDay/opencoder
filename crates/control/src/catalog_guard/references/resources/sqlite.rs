use super::wal;
use anyhow::{bail, ensure, Result};
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

/// Query only an isolated copy; the SQLite connection never opens the source files.
/// WAL frames are validated and copied; the derived WAL index is rebuilt privately.
pub(super) fn strings(source: &Path, bytes: &[u8]) -> Result<Vec<String>> {
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
    // SQLite resource files may use UTF-16, which the application database
    // engine does not support. This reader stays inside the existing blocking
    // resource scan. SQLite connections only open the copy, never a live database.
    let connection =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")?;
    let mut statement = connection.prepare("PRAGMA quick_check")?;
    let mut check = statement.query([])?;
    let row = check
        .next()?
        .ok_or_else(|| anyhow::anyhow!("empty database validation"))?;
    ensure!(
        row.get::<_, String>(0)? == "ok" && check.next()?.is_none(),
        "invalid resource database"
    );
    let mut statement = connection.prepare("SELECT name,sql,type FROM sqlite_schema")?;
    let mut schema = statement.query([])?;
    let mut tables = Vec::new();
    let mut found = Vec::new();
    while let Some(row) = schema.next()? {
        let name = row.get::<_, String>(0)?;
        let definition = row.get::<_, Option<String>>(1)?.unwrap_or_default();
        ensure!(
            !definition
                .to_ascii_uppercase()
                .contains("CREATE VIRTUAL TABLE"),
            "unverified virtual resource table"
        );
        found.push(name.clone());
        found.push(definition);
        if row.get::<_, String>(2)? == "table" {
            tables.push(name);
        }
    }
    let mut total_bytes = found.iter().map(String::len).sum::<usize>();
    let mut total_rows = 0;
    for name in tables {
        let query = format!("SELECT * FROM \"{}\"", name.replace('"', "\"\""));
        let mut statement = connection.prepare(&query)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            total_rows += 1;
            ensure!(
                total_rows <= 100_000,
                "resource database row limit exceeded"
            );
            for index in 0..row.as_ref().column_count() {
                let text = match row.get::<_, rusqlite::types::Value>(index)? {
                    rusqlite::types::Value::Text(text) => Some(text),
                    rusqlite::types::Value::Blob(bytes) => {
                        Some(String::from_utf8(bytes).map_err(|_| {
                            anyhow::anyhow!("unverified binary resource database value")
                        })?)
                    }
                    rusqlite::types::Value::Null
                    | rusqlite::types::Value::Integer(_)
                    | rusqlite::types::Value::Real(_) => None,
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
