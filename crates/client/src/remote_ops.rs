//! Remote method extensions, split out of `remote.rs` to keep file sizes
//! bounded. These cover the server's session-lifecycle and question/input
//! management endpoints (fork/compact/handoff/skill/annotation/autopilot,
//! models/skills/config listings, pending questions, queued/steer inputs).
//! All methods mirror the house style: bearer auth, `ensure_ok` for non-2xx
//! (error carries the server's JSON body), `.context()` on transport failures.

use anyhow::{Context, Result};

use crate::remote::{ensure_ok, Remote};

impl Remote {
    /// GET /api/sessions/:id → the session resource JSON (meta / messages,
    /// per the server's current shape). 404 surfaces the server error body.
    pub async fn get_session(&self, id: &str) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(self.url(&format!("/api/sessions/{id}")))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("get session")?;
        let resp = ensure_ok(resp, "get session").await?;
        resp.json().await.context("get session json")
    }

    /// DELETE /api/sessions/:id (cascades to messages/inputs/events/tasks).
    pub async fn delete_session(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .delete(self.url(&format!("/api/sessions/{id}")))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("delete session")?;
        let _ = ensure_ok(resp, "delete session").await?;
        Ok(())
    }

    /// POST /api/sessions/:id/fork → the new session's id.
    pub async fn fork_session(&self, id: &str) -> Result<String> {
        let resp = self
            .http
            .post(self.url(&format!("/api/sessions/{id}/fork")))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("fork session")?;
        let resp = ensure_ok(resp, "fork session").await?;
        let v: serde_json::Value = resp.json().await.context("fork session json")?;
        v.get("id")
            .and_then(|i| i.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("fork session: missing id in response"))
    }

    /// POST /api/sessions/:id/compact — queue a manual compaction (202).
    pub async fn post_compact(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .post(self.url(&format!("/api/sessions/{id}/compact")))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("post compact")?;
        let _ = ensure_ok(resp, "post compact").await?;
        Ok(())
    }

    /// POST /api/sessions/:id/handoff — plan→act handoff with optional extra
    /// guidance text.
    pub async fn post_handoff(&self, id: &str, extra: &str) -> Result<()> {
        let body = serde_json::json!({ "extra": extra });
        let resp = self
            .http
            .post(self.url(&format!("/api/sessions/{id}/handoff")))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("post handoff")?;
        let _ = ensure_ok(resp, "post handoff").await?;
        Ok(())
    }

    /// POST /api/sessions/:id/skill — set (`Some`) or clear (`None`) the
    /// session's active skill.
    pub async fn post_skill(&self, id: &str, skill: Option<&str>) -> Result<()> {
        let body = serde_json::json!({ "skill": skill });
        let resp = self
            .http
            .post(self.url(&format!("/api/sessions/{id}/skill")))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("post skill")?;
        let _ = ensure_ok(resp, "post skill").await?;
        Ok(())
    }

    /// POST /api/sessions/:id/annotation — set (or clear with `None`) the
    /// session's requirement annotation. Returns `{"ok":true,"requirement":..}`.
    pub async fn post_annotation(&self, id: &str, text: Option<&str>) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "text": text });
        let resp = self
            .http
            .post(self.url(&format!("/api/sessions/{id}/annotation")))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("post annotation")?;
        let resp = ensure_ok(resp, "post annotation").await?;
        resp.json().await.context("post annotation json")
    }

    /// POST /api/sessions/:id/autopilot — set (or clear with `None`) the
    /// session-scoped autopilot mode (`off|ap|review`). Returns
    /// `{"ok":true,"mode":..}`.
    pub async fn post_autopilot(&self, id: &str, mode: Option<&str>) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "mode": mode });
        let resp = self
            .http
            .post(self.url(&format!("/api/sessions/{id}/autopilot")))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("post autopilot")?;
        let resp = ensure_ok(resp, "post autopilot").await?;
        resp.json().await.context("post autopilot json")
    }

    /// GET /api/models → `{"default":..,"models":[..],"providers":[..]}`.
    pub async fn get_models(&self) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(self.url("/api/models"))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("get models")?;
        let resp = ensure_ok(resp, "get models").await?;
        resp.json().await.context("get models json")
    }

    /// GET /api/skills → `{"skills":[{"name","description","enabled"}]}`.
    pub async fn get_skills(&self) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(self.url("/api/skills"))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("get skills")?;
        let resp = ensure_ok(resp, "get skills").await?;
        resp.json().await.context("get skills json")
    }

    /// GET /api/config → the redacted merged config JSON.
    pub async fn get_config(&self) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(self.url("/api/config"))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("get config")?;
        let resp = ensure_ok(resp, "get config").await?;
        resp.json().await.context("get config json")
    }

    /// PATCH /api/config — merge a JSON patch into the config; the server
    /// saves + reloads and echoes `{"ok":true}` (or a 4xx on a bad patch).
    pub async fn patch_config(&self, patch: serde_json::Value) -> Result<serde_json::Value> {
        let resp = self
            .http
            .patch(self.url("/api/config"))
            .bearer_auth(&self.token)
            .json(&patch)
            .send()
            .await
            .context("patch config")?;
        let resp = ensure_ok(resp, "patch config").await?;
        resp.json().await.context("patch config json")
    }

    /// GET /api/sessions/:id/questions → pending question cards
    /// (`[{"id","question","options":[..]}]`).
    pub async fn list_questions(&self, id: &str) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .http
            .get(self.url(&format!("/api/sessions/{id}/questions")))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("list questions")?;
        let resp = ensure_ok(resp, "list questions").await?;
        let v: serde_json::Value = resp.json().await.context("list questions json")?;
        Ok(v.get("questions")
            .cloned()
            .and_then(|q| serde_json::from_value(q).ok())
            .unwrap_or_default())
    }

    /// POST /api/sessions/:id/questions/:call_id/answer — deliver the user's
    /// answer to a pending question. 404 (already answered/skipped/vanished)
    /// surfaces the server's error body.
    pub async fn answer_question(&self, id: &str, call_id: &str, answer: &str) -> Result<()> {
        let body = serde_json::json!({ "answer": answer });
        let resp = self
            .http
            .post(self.url(&format!("/api/sessions/{id}/questions/{call_id}/answer")))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("answer question")?;
        let _ = ensure_ok(resp, "answer question").await?;
        Ok(())
    }

    /// POST /api/sessions/:id/questions/:call_id/skip — skip a pending
    /// question (the model proceeds with its best judgment).
    pub async fn skip_question(&self, id: &str, call_id: &str) -> Result<()> {
        let resp = self
            .http
            .post(self.url(&format!("/api/sessions/{id}/questions/{call_id}/skip")))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("skip question")?;
        let _ = ensure_ok(resp, "skip question").await?;
        Ok(())
    }

    /// GET /api/sessions/:id/inputs?delivery=queue|steer → the session's
    /// pending inputs for that delivery lane
    /// (`[{"seq","delivery","prompt","admitted_seq","promoted_seq","images"}]`).
    pub async fn list_inputs(&self, id: &str, delivery: &str) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .http
            .get(self.url(&format!("/api/sessions/{id}/inputs")))
            .query(&[("delivery", delivery)])
            .bearer_auth(&self.token)
            .send()
            .await
            .context("list inputs")?;
        let resp = ensure_ok(resp, "list inputs").await?;
        let v: serde_json::Value = resp.json().await.context("list inputs json")?;
        Ok(v.get("inputs")
            .cloned()
            .and_then(|i| serde_json::from_value(i).ok())
            .unwrap_or_default())
    }

    /// DELETE /api/sessions/:id/inputs/:seq — drop a pending input by seq.
    pub async fn delete_input(&self, id: &str, seq: i64) -> Result<()> {
        let resp = self
            .http
            .delete(self.url(&format!("/api/sessions/{id}/inputs/{seq}")))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("delete input")?;
        let _ = ensure_ok(resp, "delete input").await?;
        Ok(())
    }

    /// POST /api/sessions/:id/inputs/reorder — swap two pending inputs' order.
    pub async fn reorder_inputs(&self, id: &str, a: i64, b: i64) -> Result<()> {
        let body = serde_json::json!({ "a": a, "b": b });
        let resp = self
            .http
            .post(self.url(&format!("/api/sessions/{id}/inputs/reorder")))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("reorder inputs")?;
        let _ = ensure_ok(resp, "reorder inputs").await?;
        Ok(())
    }

    /// GET /api/sessions/:id/subagents — the session's subagent task records
    /// (`[{"id","kind","status","child_session_id","prompt",...}]`). 404 when
    /// the parent session does not exist (empty list is a normal 200).
    pub async fn list_subagents(&self, id: &str) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .http
            .get(self.url(&format!("/api/sessions/{id}/subagents")))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("list subagents")?;
        let resp = ensure_ok(resp, "list subagents").await?;
        let v: serde_json::Value = resp.json().await.context("list subagents json")?;
        Ok(v.get("tasks")
            .cloned()
            .and_then(|t| serde_json::from_value(t).ok())
            .unwrap_or_default())
    }

    /// DELETE /api/sessions?keep=<id> — clear every remote session except
    /// `keep` (FK-cascades to messages/inputs/events/subagent tasks). Returns
    /// the removed count. 409 while any session drain is running.
    pub async fn clear_sessions(&self, keep: &str) -> Result<u64> {
        let resp = self
            .http
            .delete(self.url("/api/sessions"))
            .query(&[("keep", keep)])
            .bearer_auth(&self.token)
            .send()
            .await
            .context("clear sessions")?;
        let resp = ensure_ok(resp, "clear sessions").await?;
        let v: serde_json::Value = resp.json().await.context("clear sessions json")?;
        Ok(v.get("removed").and_then(|r| r.as_u64()).unwrap_or(0))
    }
}
