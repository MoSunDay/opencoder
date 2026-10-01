use super::*;

#[tokio::test]
async fn mysql_project_crud_contract() {
    let Some(dsn) = env_dsn("OC_TEST_MYSQL_DSN") else {
        eprintln!("warning: OC_TEST_MYSQL_DSN not set — skipping sql_store integration test");
        return;
    };
    let p = sql_store::open(&storage(StorageBackend::Mysql, &dsn))
        .await
        .expect("open mysql project store");
    crud_contract(p.as_ref(), &ulid::Ulid::new().to_string(), "mysql").await;
}

#[cfg(feature = "starrocks")]
#[tokio::test]
async fn starrocks_project_crud_contract() {
    let Some(dsn) = env_dsn("OC_TEST_STARROCKS_DSN") else {
        eprintln!("warning: OC_TEST_STARROCKS_DSN not set — skipping sql_store integration test");
        return;
    };
    let p = sql_store::open(&storage(StorageBackend::Starrocks, &dsn))
        .await
        .expect("open starrocks project store");
    crud_contract(p.as_ref(), &ulid::Ulid::new().to_string(), "starrocks").await;
}
