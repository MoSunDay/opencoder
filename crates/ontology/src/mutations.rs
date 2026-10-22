use crate::{
    database::{Database, REQUEST_CONNECTION},
    error::AppError,
    http::AppState,
};
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::atomic::Ordering;

pub(crate) struct Active(Database);
impl Active {
    pub(crate) fn new(database: Database) -> Self {
        database.active.fetch_add(1, Ordering::SeqCst);
        Self(database)
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

pub(crate) async fn transaction(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let write = !matches!(request.method().as_str(), "GET" | "HEAD")
        && !request.uri().path().ends_with("/vector-search");
    let active = Active::new(state.database.clone());
    // Detaching from the client preserves accepted mutations; Server retirement
    // waits for Active guards, including requests whose client already left.
    let task = tokio::spawn(async move {
        let _active = active;
        let database = state.database.clone();
        let work = move |connection: turso::Connection| async move {
            if write {
                connection.execute("BEGIN IMMEDIATE", ()).await?;
            }
            let response = REQUEST_CONNECTION
                .scope(connection.clone(), async {
                    if write {
                        if let Err(error) = state
                            .database
                            .store()
                            .await
                            .catalog_guard(request.uri().path())
                            .await
                        {
                            return error.into_response();
                        }
                    }
                    next.run(request).await
                })
                .await;
            if write {
                let finish = if response.status().is_success() {
                    "COMMIT"
                } else {
                    "ROLLBACK"
                };
                connection.execute(finish, ()).await?;
            }
            Ok::<_, anyhow::Error>(response)
        };
        let result = if write {
            database.db.write(work).await
        } else {
            database.db.read(work).await
        };
        match result {
            Ok(response) => response,
            Err(error) => AppError::from(error).into_response(),
        }
    });
    match task.await {
        Ok(response) => response,
        Err(error) => AppError::dependency(error.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, middleware, routing::post, Router};
    use std::sync::Arc;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    #[tokio::test]
    async fn disconnected_clients_do_not_cancel_accepted_writes_and_shutdown_waits() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(&dir.path().join("ontology.db"), &dir.path().join("files"))
            .await
            .unwrap();
        let entered = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let writer = state.clone();
        let notify = entered.clone();
        let proceed = resume.clone();
        let app = Router::new()
            .route(
                "/write",
                post(move || async move {
                    notify.notify_one();
                    proceed.notified().await;
                    writer
                        .database
                        .store()
                        .await
                        .create_environment("accepted", "Accepted", "", "test")
                        .await
                        .unwrap();
                }),
            )
            .layer(middleware::from_fn_with_state(state.clone(), transaction));
        let client = tokio::spawn(
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/write")
                    .body(Body::empty())
                    .unwrap(),
            ),
        );
        entered.notified().await;
        client.abort();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), state.drained())
                .await
                .is_err()
        );
        resume.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(5), state.drained())
            .await
            .unwrap();
        assert_eq!(
            state
                .database
                .store()
                .await
                .environment("accepted")
                .await
                .unwrap()
                .env_key,
            "accepted"
        );
    }
}
