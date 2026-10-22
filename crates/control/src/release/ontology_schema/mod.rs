//! Authorize shared schema changes only inside a stopped, backed-up release.
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::release::PlatformConfig;
use opencoder_ontology::SchemaPolicy;
use serde_json::Value;
use std::{fs::File, path::Path};

#[cfg(test)]
mod tests;

pub(crate) struct Startup {
    pub policy: SchemaPolicy,
    // Journal.save takes the exclusive lock: permission remains valid through DDL.
    _lock: Option<File>,
}

pub(crate) async fn startup(
    platform: Option<&PlatformConfig>,
    port: u16,
    data: &Path,
) -> Result<Startup> {
    let Some(platform) = platform.cloned() else {
        return Ok(Startup {
            policy: SchemaPolicy::InitializeEmpty,
            _lock: None,
        });
    };
    let data = data.to_owned();
    tokio::task::spawn_blocking(move || check(&platform, port, &data)).await?
}

fn check(platform: &PlatformConfig, port: u16, data: &Path) -> Result<Startup> {
    let lock = File::open(platform.state_dir.join("catalog-writer.lock"))
        .context("release schema authorization requires the catalog lock")?;
    fs2::FileExt::lock_shared(&lock)?;
    let journal: Value = serde_json::from_slice(&std::fs::read(
        platform.state_dir.join("release-state.json"),
    )?)?;
    ensure!(
        journal["schema_version"] == 1,
        "unsupported release journal"
    );
    let record = &journal["releases"][&platform.release_id];
    ensure!(
        port != 0 && record["server_port"].as_u64() == Some(u64::from(port)),
        "release Server port differs from schema authorization"
    );
    ensure!(
        data.canonicalize()? == platform.state_dir.join("server").canonicalize()?,
        "release Server data differs from managed shared storage"
    );
    let maintenance = &journal["maintenance"];
    let initial = journal["phase"] == "migrating"
        && journal["migration_stage"] == "services"
        && journal["current"].is_null()
        && journal["candidate"] == platform.release_id;
    let upgrading = journal["phase"] == "migrating"
        && maintenance["stage"] == "installing"
        && maintenance["target"] == platform.release_id
        && maintenance["writes_open"] == false
        && maintenance["schema_started"] == true;
    if !initial && !upgrading {
        return Ok(Startup {
            policy: SchemaPolicy::Validate,
            _lock: Some(lock),
        });
    }
    let proof = if initial {
        &journal["ontology_initial_migration"]
    } else {
        &maintenance["ontology_migration"]
    };
    ensure!(
        proof["schema"] == 1
            && proof["release_id"] == platform.release_id
            && proof["server_port"] == port,
        "Ontology migration permission is missing or mismatched"
    );
    ensure!(
        Path::new(
            proof["database"]
                .as_str()
                .context("missing migration database")?
        ) == data.canonicalize()?.join("ontology.db"),
        "Ontology migration database differs"
    );
    // The controller verifies every backup file before journaling this proof.
    // Server must not acquire access to root-owned backup/configuration files.
    let digest = proof["backup_sha256"]
        .as_str()
        .context("missing verified backup digest")?;
    ensure!(
        proof["backup_verified"] == true
            && digest.len() == 64
            && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Ontology migration requires a verified backup receipt"
    );
    Ok(Startup {
        policy: SchemaPolicy::Maintenance,
        _lock: Some(lock),
    })
}
