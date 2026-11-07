//! `ProjectStore for RdbStore`: thin async delegations. Each method owns its
//! inputs (clone + move) because the closure crosses threads, then hands the
//! engine handle to the matching free function under the read path or the
//! exclusive write lease. All semantics live in the free functions.

use anyhow::Result;

use crate::project::comments::{Comment, CommentMutation, CommentPage, CommentQuery, NewComment};
use crate::project::summary::{SummaryScope, SummaryVersion};
use crate::project::ProjectStore;
use crate::project::{ProjectAssignment, ProjectTag, ProjectTodoTag};
use crate::project_types::{
    ProjectGoalPatch, ProjectGoalRecord, ProjectInitiativePatch, ProjectInitiativeRecord,
    ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunPatch, ProjectTodoRunRecord,
    ProjectTodoRunStatus, ProjectTodoRunSummary, ProjectTodoStatus, ProjectTodoSummary,
};
use crate::rdb_store::projects::{
    comments, goals, initiatives, links, runs, runs_inspect, summary, tags, todos,
};
use crate::rdb_store::RdbStore;
use crate::{PayloadChunkRecord, ProjectTodoRunPage};

#[async_trait::async_trait]
impl ProjectStore for RdbStore {
    async fn todo_comments(&self, id: &str, query: CommentQuery) -> Result<Option<CommentPage>> {
        let id = id.to_owned();
        self.db.read(move |db| comments::page(db, &id, query)).await
    }
    async fn todo_comment_request(
        &self,
        id: &str,
        user: &str,
        request: &str,
    ) -> Result<Option<Comment>> {
        let (id, user, request) = (id.to_owned(), user.to_owned(), request.to_owned());
        self.db
            .read(move |db| comments::request(db, &id, &user, &request))
            .await
    }
    async fn append_todo_comment(&self, entry: &NewComment) -> Result<Comment> {
        let entry = entry.clone();
        self.db.write(move |db| comments::append(db, &entry)).await
    }
    async fn mutate_todo_comment(&self, change: &CommentMutation) -> Result<Option<Comment>> {
        let change = change.clone();
        self.db.write(move |db| comments::mutate(db, &change)).await
    }
    async fn summary_versions(
        &self,
        scope: SummaryScope,
        id: &str,
    ) -> Result<Option<Vec<SummaryVersion>>> {
        let id = id.to_owned();
        self.db.read(move |db| summary::list(db, scope, &id)).await
    }
    async fn restore_summary(
        &self,
        scope: SummaryScope,
        id: &str,
        version: i64,
        now: i64,
    ) -> Result<Option<SummaryVersion>> {
        let id = id.to_owned();
        self.db
            .write(move |db| summary::restore(db, scope, &id, version, now))
            .await
    }
    async fn list_tags(&self) -> Result<Vec<ProjectTag>> {
        self.db.read(tags::list).await
    }
    async fn list_todo_tags(&self) -> Result<Vec<ProjectTodoTag>> {
        self.db.read(tags::links).await
    }
    async fn write_tag(&self, tag: &ProjectTag) -> Result<()> {
        let tag = tag.clone();
        self.db.write(move |db| tags::write(db, &tag)).await
    }
    async fn delete_tag(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.db.write(move |db| tags::delete(db, &id)).await
    }
    async fn create_todo_tagged(&self, rec: &ProjectTodoRecord, ids: &[String]) -> Result<()> {
        let (rec, ids) = (rec.clone(), ids.to_vec());
        self.db
            .write(move |db| tags::create_todo(db, &rec, &ids))
            .await
    }
    async fn patch_todo_tagged(
        &self,
        id: &str,
        patch: &ProjectTodoPatch,
        ids: Option<&[String]>,
        now: i64,
    ) -> Result<bool> {
        let (id, patch, ids) = (id.to_owned(), patch.clone(), ids.map(<[String]>::to_vec));
        self.db
            .write(move |db| tags::patch_todo(db, &id, &patch, ids.as_deref(), None, now))
            .await
    }
    async fn patch_todo_accepted(
        &self,
        id: &str,
        patch: &ProjectTodoPatch,
        ids: Option<&[String]>,
        expected: &crate::project::acceptance::TodoAcceptance,
        now: i64,
    ) -> Result<bool> {
        let (id, patch, ids, expected) = (
            id.to_owned(),
            patch.clone(),
            ids.map(<[String]>::to_vec),
            expected.clone(),
        );
        self.db
            .write(move |db| {
                tags::patch_todo(db, &id, &patch, ids.as_deref(), Some(&expected), now)
            })
            .await
    }
    async fn create_goal(&self, rec: &ProjectGoalRecord) -> Result<()> {
        let rec = rec.clone();
        self.db.write(move |db| goals::create(db, &rec)).await
    }
    async fn patch_goal(&self, id: &str, patch: &ProjectGoalPatch, now_ms: i64) -> Result<bool> {
        let (id, patch) = (id.to_owned(), patch.clone());
        self.db
            .write(move |db| goals::patch(db, &id, &patch, now_ms))
            .await
    }
    async fn delete_goal(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.db.write(move |db| goals::delete(db, &id)).await
    }
    async fn list_goals(&self) -> Result<Vec<ProjectGoalRecord>> {
        self.db.read(goals::list).await
    }
    async fn create_initiative(&self, rec: &ProjectInitiativeRecord) -> Result<()> {
        let rec = rec.clone();
        self.db.write(move |db| initiatives::create(db, &rec)).await
    }
    async fn patch_initiative(
        &self,
        id: &str,
        patch: &ProjectInitiativePatch,
        now_ms: i64,
    ) -> Result<bool> {
        let (id, patch) = (id.to_owned(), patch.clone());
        self.db
            .write(move |db| initiatives::patch(db, &id, &patch, now_ms))
            .await
    }
    async fn delete_initiative(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.db.write(move |db| initiatives::delete(db, &id)).await
    }
    async fn list_initiatives(
        &self,
        goal_id: Option<&str>,
    ) -> Result<Vec<ProjectInitiativeRecord>> {
        let goal_id = goal_id.map(str::to_owned);
        self.db
            .read(move |db| initiatives::list(db, goal_id.as_deref()))
            .await
    }
    async fn reorder_initiatives(
        &self,
        goal_id: &str,
        expected_ids: &[String],
        ids: &[String],
        now_ms: i64,
    ) -> Result<bool> {
        let (goal_id, expected_ids, ids) =
            (goal_id.to_owned(), expected_ids.to_vec(), ids.to_vec());
        self.db
            .write(move |db| initiatives::reorder(db, &goal_id, &expected_ids, &ids, now_ms))
            .await
    }
    async fn create_todo(&self, rec: &ProjectTodoRecord) -> Result<()> {
        let rec = rec.clone();
        self.db
            .write(move |db| tags::create_todo(db, &rec, &[]))
            .await
    }
    async fn patch_todo(&self, id: &str, patch: &ProjectTodoPatch, now_ms: i64) -> Result<bool> {
        let (id, patch) = (id.to_owned(), patch.clone());
        self.db
            .write(move |db| tags::patch_todo(db, &id, &patch, None, None, now_ms))
            .await
    }
    async fn reorder_todos(
        &self,
        initiative_id: Option<&str>,
        board_status: &str,
        ids: &[String],
        now_ms: i64,
    ) -> Result<()> {
        let (initiative_id, board_status, ids) = (
            initiative_id.map(str::to_owned),
            board_status.to_owned(),
            ids.to_vec(),
        );
        self.db
            .write(move |db| {
                todos::reorder(db, initiative_id.as_deref(), &board_status, &ids, now_ms)
            })
            .await
    }
    async fn claim_todo_running(&self, id: &str, now_ms: i64) -> Result<bool> {
        let id = id.to_owned();
        self.db
            .write(move |db| todos::claim_running(db, &id, now_ms))
            .await
    }
    async fn claim_todo_running_with_run(
        &self,
        rec: &ProjectTodoRunRecord,
        now_ms: i64,
    ) -> Result<bool> {
        let rec = rec.clone();
        self.db
            .write(move |db| todos::claim_running_with_run(db, &rec, now_ms))
            .await
    }
    async fn patch_todo_when(
        &self,
        id: &str,
        when: ProjectTodoStatus,
        patch: &ProjectTodoPatch,
        now_ms: i64,
    ) -> Result<bool> {
        let (id, patch) = (id.to_owned(), patch.clone());
        self.db
            .write(move |db| todos::patch_when(db, &id, when, &patch, now_ms))
            .await
    }
    async fn delete_todo(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.db.write(move |db| todos::delete(db, &id)).await
    }
    async fn get_todo(&self, id: &str) -> Result<Option<ProjectTodoRecord>> {
        let id = id.to_owned();
        self.db.read(move |db| todos::get(db, &id)).await
    }
    async fn get_todo_summary(&self, id: &str) -> Result<Option<ProjectTodoSummary>> {
        let id = id.to_owned();
        self.db.read(move |db| todos::get_summary(db, &id)).await
    }
    async fn list_todos(&self, initiative_id: Option<&str>) -> Result<Vec<ProjectTodoRecord>> {
        let initiative_id = initiative_id.map(str::to_owned);
        self.db
            .read(move |db| todos::list(db, initiative_id.as_deref()))
            .await
    }
    async fn list_todo_assignments(&self, todo_id: &str) -> Result<Vec<ProjectAssignment>> {
        let todo_id = todo_id.to_owned();
        self.db.read(move |db| links::list(db, &todo_id)).await
    }
    async fn latest_todo_assignments(&self) -> Result<Vec<ProjectAssignment>> {
        self.db.read(links::latest).await
    }
    async fn link_todo_execution(&self, assignment: &ProjectAssignment) -> Result<()> {
        let assignment = assignment.clone();
        self.db.write(move |db| links::link(db, &assignment)).await
    }
    async fn unlink_todo_execution(&self, todo_id: &str, execution_id: &str) -> Result<bool> {
        let (todo_id, execution_id) = (todo_id.to_owned(), execution_id.to_owned());
        self.db
            .write(move |db| links::unlink(db, &todo_id, &execution_id))
            .await
    }
    async fn create_todo_run(&self, rec: &ProjectTodoRunRecord) -> Result<()> {
        let rec = rec.clone();
        self.db.write(move |db| runs::create(db, &rec)).await
    }
    async fn finish_todo_run(
        &self,
        id: &str,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool> {
        let (id, patch) = (id.to_owned(), patch.clone());
        self.db
            .write(move |db| runs::finish(db, &id, &patch, now_ms))
            .await
    }
    async fn patch_todo_run(
        &self,
        id: &str,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool> {
        let (id, patch) = (id.to_owned(), patch.clone());
        self.db
            .write(move |db| runs::patch(db, &id, &patch, now_ms))
            .await
    }
    async fn patch_todo_run_when(
        &self,
        id: &str,
        when: ProjectTodoRunStatus,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool> {
        let (id, patch) = (id.to_owned(), patch.clone());
        self.db
            .write(move |db| runs::patch_when(db, &id, when, &patch, now_ms))
            .await
    }
    async fn get_todo_run(&self, id: &str) -> Result<Option<ProjectTodoRunRecord>> {
        let id = id.to_owned();
        self.db.read(move |db| runs::get(db, &id)).await
    }
    async fn get_todo_run_summary(&self, id: &str) -> Result<Option<ProjectTodoRunSummary>> {
        let id = id.to_owned();
        self.db
            .read(move |db| runs_inspect::get_summary(db, &id))
            .await
    }
    async fn list_todo_runs(&self, todo_id: &str) -> Result<Vec<ProjectTodoRunRecord>> {
        let todo_id = todo_id.to_owned();
        self.db
            .read(move |db| runs_inspect::list(db, &todo_id))
            .await
    }
    async fn list_todo_runs_page(
        &self,
        todo_id: &str,
        before_version: Option<i64>,
        limit: u32,
    ) -> Result<ProjectTodoRunPage> {
        let todo_id = todo_id.to_owned();
        self.db
            .read(move |db| runs_inspect::list_page(db, &todo_id, before_version, limit))
            .await
    }
    async fn project_text_chunk(
        &self,
        record_kind: &str,
        owner_id: &str,
        id: &str,
        field: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<PayloadChunkRecord>> {
        let (record_kind, owner_id, id, field) = (
            record_kind.to_owned(),
            owner_id.to_owned(),
            id.to_owned(),
            field.to_owned(),
        );
        self.db
            .read(move |db| {
                runs_inspect::text_chunk(
                    db,
                    &record_kind,
                    &owner_id,
                    &id,
                    &field,
                    offset,
                    max_bytes,
                )
            })
            .await
    }
    async fn list_running_todo_runs(&self) -> Result<Vec<ProjectTodoRunRecord>> {
        self.db.read(runs_inspect::list_running).await
    }
    async fn next_todo_version(&self, todo_id: &str) -> Result<i64> {
        let todo_id = todo_id.to_owned();
        self.db
            .read(move |db| Ok(runs_inspect::next_version(db, &todo_id)))
            .await
    }
}
