//! Caller owns the write transaction, including the object and tag changes.
use crate::project::people::{InvalidProjectUser, ProjectPeople, ProjectPeoplePatch};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use turso::{params, Connection};

pub(crate) async fn list(
    conn: &Connection,
    kind: &str,
    id: Option<&str>,
) -> Result<BTreeMap<String, ProjectPeople>> {
    let mut rows = conn
        .query(
            "SELECT object_id,user_name,relation FROM project_people
        WHERE object_kind=?1 AND (?2 IS NULL OR object_id=?2) ORDER BY object_id,user_name",
            params![kind, id],
        )
        .await?;
    let mut result = BTreeMap::<String, ProjectPeople>::new();
    while let Some(row) = rows.next().await? {
        let people = result.entry(row.get(0)?).or_default();
        let name = row.get(1)?;
        match row.get::<String>(2)?.as_str() {
            "owner" => people.owner_user_name = Some(name),
            "follower" => people.follower_user_names.push(name),
            _ => anyhow::bail!("invalid project people relation"),
        }
    }
    Ok(result)
}

pub(crate) async fn patch(
    conn: &Connection,
    kind: &str,
    id: &str,
    patch: &ProjectPeoplePatch,
) -> Result<()> {
    let owner = patch
        .owner_user_name
        .as_ref()
        .and_then(|value| value.as_deref());
    let names: BTreeSet<&str> = owner
        .into_iter()
        .chain(
            patch
                .follower_user_names
                .iter()
                .flatten()
                .map(String::as_str),
        )
        .collect();
    for name in names {
        let mut rows = conn
            .query("SELECT 1 FROM platform_users WHERE name=?", params![name])
            .await?;
        anyhow::ensure!(
            rows.next().await?.is_some(),
            InvalidProjectUser(name.into())
        );
    }
    let owner_names = owner.map(str::to_owned).into_iter().collect::<Vec<_>>();
    for (relation, names) in [
        (
            "owner",
            patch
                .owner_user_name
                .as_ref()
                .map(|_| owner_names.as_slice()),
        ),
        ("follower", patch.follower_user_names.as_deref()),
    ] {
        let Some(names) = names else {
            continue;
        };
        conn.execute(
            "DELETE FROM project_people WHERE object_kind=? AND object_id=? AND relation=?",
            params![kind, id, relation],
        )
        .await?;
        for name in names.iter().collect::<BTreeSet<_>>() {
            conn.execute("INSERT INTO project_people(object_kind,object_id,user_name,relation) VALUES (?,?,?,?)", params![kind,id,name.as_str(),relation]).await?;
        }
    }
    Ok(())
}
