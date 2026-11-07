use opencoder_store::{ProjectStore, TursoStore};
use serde_json::json;

pub async fn seed(store: &TursoStore) {
    store
        .create_goal(
            &serde_json::from_value(json!({
                "id":"p", "title":"Project", "status":"active", "sort":0,
                "created_at":1, "updated_at":1
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    store
        .create_initiative(
            &serde_json::from_value(json!({
                "id":"i", "goal_id":"p", "title":"Initiative", "sort":0,
                "created_at":1, "updated_at":1
            }))
            .unwrap(),
        )
        .await
        .unwrap();
}
