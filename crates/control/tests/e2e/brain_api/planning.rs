use super::*;
#[tokio::test]
async fn legacy_planning_and_dispatch_are_migration_errors_without_fallback() {
 let h=Harness::new().await;
 for path in ["/api/brain/plans","/api/brain/preview","/api/brain/dispatch","/api/brain/playbooks/trigger-scan","/api/brain/playbooks/old/dispatch"] {
  let(status,body)=h.req(Method::POST,path,Some(json!({"situation":"do work","text":"work"}))).await;
  assert_eq!(status,409,"{path}: {body}");assert!(body.to_string().contains("migration required"));
 }
 assert_eq!(h.mock_llm.call_count(),0);
}
