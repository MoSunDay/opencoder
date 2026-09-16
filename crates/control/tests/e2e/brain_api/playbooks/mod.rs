use reqwest::Method;
use serde_json::json;
use crate::support::Harness;
#[tokio::test]
async fn historical_playbook_is_queryable_but_cannot_be_dispatched() {
 let h=Harness::new().await;
 let record=opencoder_store::BrainPlaybookRecord{id:"legacy".into(),name:"legacy".into(),origin:"fixed".into(),situation_digest:None,spec_json:"{\"schema_version\":1}".into(),created_at:1,updated_at:1};
 h.state.store.save_brain_playbook(&record).await.unwrap();
 let(status,body)=h.req(Method::GET,"/api/brain/playbooks/legacy",None).await;assert_eq!(status,200);assert_eq!(body["spec_json"],record.spec_json);
 let(status,_)=h.req(Method::POST,"/api/brain/playbooks/legacy/dispatch",Some(json!({"situation":"legacy"}))).await;assert_eq!(status,409);
 assert_eq!(h.state.store.get_brain_playbook("legacy").await.unwrap().unwrap().spec_json,record.spec_json);
}
