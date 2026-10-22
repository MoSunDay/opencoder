macro_rules! project_implementation_0 {
    ($($methods:tt)*) => {
        project_implementation_1! {
            $($methods)*
    async fn todo_comments(
        &self,
        id: &str,
        query: crate::project::comments::CommentQuery,
    ) -> Result<Option<crate::project::comments::CommentPage>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        comments::page(&conn, id, query).await

        }).await
    }
    async fn todo_comment_request(
        &self,
        id: &str,
        user: &str,
        request: &str,
    ) -> Result<Option<crate::project::comments::Comment>> {
        let id = id.to_owned();
        let user = user.to_owned();
        let request = request.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();
            let user = user.as_str();
            let request = request.as_str();

        comments::request(&conn, id, user, request).await

        }).await
    }
    async fn append_todo_comment(
        &self,
        entry: &crate::project::comments::NewComment,
    ) -> Result<crate::project::comments::Comment> {
        let entry = entry.to_owned();
        self.db.write(move |conn| async move {
            let entry = &entry;

        comments::append(&conn, entry).await

        }).await
    }
    async fn mutate_todo_comment(
        &self,
        change: &crate::project::comments::CommentMutation,
    ) -> Result<Option<crate::project::comments::Comment>> {
        let change = change.clone();
        self.db.write(move |conn| async move {
            comments::mutations::apply(&conn, &change).await
        }).await
    }
    async fn summary_versions(
        &self,
        scope: SummaryScope,
        id: &str,
    ) -> Result<Option<Vec<SummaryVersion>>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        summaries::list(&conn, scope, id).await

        }).await
    }
    async fn restore_summary(
        &self,
        scope: SummaryScope,
        id: &str,
        version: i64,
        now: i64,
    ) -> Result<Option<SummaryVersion>> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        summaries::restore(&conn, scope, id, version, now).await

        }).await
    }
    async fn list_tags(&self) -> Result<Vec<crate::project::ProjectTag>> {
        self.db.read(move |conn| async move {

        tags::list(&conn).await

        }).await
    }
    async fn list_todo_tags(&self) -> Result<Vec<crate::project::ProjectTodoTag>> {
        self.db.read(move |conn| async move {

        tags::links(&conn).await

        }).await
    }
    async fn write_tag(&self, tag: &crate::project::ProjectTag) -> Result<()> {
        let tag = tag.to_owned();
        self.db.write(move |conn| async move {
            let tag = &tag;

        tags::write(&conn, tag).await

        }).await
    }
    async fn delete_tag(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        tags::delete(&conn, id).await

        }).await
    }
    async fn create_todo_tagged(&self, rec: &ProjectTodoRecord, ids: &[String]) -> Result<()> {
        let rec = rec.to_owned();
        let ids = ids.to_owned();
        self.db.write(move |conn| async move {
            let rec = &rec;
            let ids = &ids;

        tags::create_todo(&conn, rec, ids).await

        }).await
    }
    async fn patch_todo_tagged(
        &self,
        id: &str,
        patch: &ProjectTodoPatch,
        ids: Option<&[String]>,
        now: i64,
    ) -> Result<bool> {
        let id = id.to_owned();
        let patch = patch.to_owned();
        let ids = ids.map(ToOwned::to_owned);
        self.db.write(move |conn| async move {
            let id = id.as_str();
            let patch = &patch;
            let ids = ids.as_deref();

        tags::patch_todo(&conn, id, patch, ids, None, now).await

        }).await
    }
    async fn patch_todo_accepted(
        &self,
        id: &str,
        patch: &ProjectTodoPatch,
        ids: Option<&[String]>,
        expected: &crate::project::acceptance::TodoAcceptance,
        now: i64,
    ) -> Result<bool> {
        let id = id.to_owned();
        let patch = patch.to_owned();
        let ids = ids.map(ToOwned::to_owned);
        let expected = expected.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();
            let patch = &patch;
            let ids = ids.as_deref();
            let expected = &expected;

        tags::patch_todo(&conn, id, patch, ids, Some(expected), now).await

        }).await
    }
    async fn list_todo_assignments(
        &self,
        todo_id: &str,
    ) -> Result<Vec<crate::project::ProjectAssignment>> {
        let todo_id = todo_id.to_owned();
        self.db.read(move |conn| async move {
            let todo_id = todo_id.as_str();

        super::project_links::list(&conn, todo_id).await

        }).await
    }
    async fn latest_todo_assignments(&self) -> Result<Vec<crate::project::ProjectAssignment>> {
        self.db.read(move |conn| async move {

        super::project_links::latest(&conn).await

        }).await
    }
    async fn link_todo_execution(
        &self,
        assignment: &crate::project::ProjectAssignment,
    ) -> Result<()> {
        let assignment = assignment.to_owned();
        self.db.write(move |conn| async move {
            let assignment = &assignment;

        super::project_links::link(&conn, assignment).await

        }).await
    }
    async fn unlink_todo_execution(&self, todo_id: &str, execution_id: &str) -> Result<bool> {
        let todo_id = todo_id.to_owned();
        let execution_id = execution_id.to_owned();
        self.db.write(move |conn| async move {
            let todo_id = todo_id.as_str();
            let execution_id = execution_id.as_str();

        super::project_links::unlink(&conn, todo_id, execution_id).await

        }).await
    }
    async fn create_goal(&self, rec: &ProjectGoalRecord) -> Result<()> {
        let rec = rec.to_owned();
        self.db.write(move |conn| async move {
            let rec = &rec;

        super::tx::run_tx(&conn, "BEGIN IMMEDIATE", || create_goal(&conn, rec)).await

        }).await
    }
    async fn patch_goal(&self, id: &str, patch: &ProjectGoalPatch, now_ms: i64) -> Result<bool> {
        let id = id.to_owned();
        let patch = patch.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();
            let patch = &patch;

        super::tx::run_tx(&conn, "BEGIN IMMEDIATE", || {
            patch_goal(&conn, id, patch, now_ms)
        })
        .await

        }).await
    }
    async fn delete_goal(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        delete_goal(&conn, id).await

        }).await
    }
    async fn list_goals(&self) -> Result<Vec<ProjectGoalRecord>> {
        self.db.read(move |conn| async move {

        list_goals(&conn).await

        }).await
    }
    async fn create_initiative(&self, rec: &ProjectInitiativeRecord) -> Result<()> {
        let rec = rec.to_owned();
        self.db.write(move |conn| async move {
            let rec = &rec;

        super::tx::run_tx(&conn, "BEGIN IMMEDIATE", || create_initiative(&conn, rec)).await

        }).await
    }
    async fn patch_initiative(
        &self,
        id: &str,
        patch: &ProjectInitiativePatch,
        now_ms: i64,
    ) -> Result<bool> {
        let id = id.to_owned();
        let patch = patch.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();
            let patch = &patch;

        super::tx::run_tx(&conn, "BEGIN IMMEDIATE", || async {
            let changed = patch_initiative(&conn, id, patch, now_ms).await?;
            if changed && patch.goal_id.is_some() {
                tags::reconcile(&conn, &[], None).await?;
            }
            Ok(changed)
        })
        .await

        }).await
    }
    async fn delete_initiative(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        delete_initiative(&conn, id).await

        }).await
    }
    async fn list_initiatives(
        &self,
        goal_id: Option<&str>,
    ) -> Result<Vec<ProjectInitiativeRecord>> {
        let goal_id = goal_id.map(ToOwned::to_owned);
        self.db.read(move |conn| async move {
            let goal_id = goal_id.as_deref();

        list_initiatives(&conn, goal_id).await

        }).await
    }
    async fn reorder_initiatives(
        &self,
        goal_id: &str,
        expected_ids: &[String],
        ids: &[String],
        now_ms: i64,
    ) -> Result<bool> {
        let goal_id = goal_id.to_owned();
        let expected_ids = expected_ids.to_owned();
        let ids = ids.to_owned();
        self.db.write(move |conn| async move {
            let goal_id = goal_id.as_str();
            let expected_ids = &expected_ids;
            let ids = &ids;

        order::reorder(&conn, goal_id, expected_ids, ids, now_ms).await

        }).await
    }
    async fn create_todo(&self, rec: &ProjectTodoRecord) -> Result<()> {
        let rec = rec.to_owned();
        self.db.write(move |conn| async move {
            let rec = &rec;

        tags::create_todo(&conn, rec, &[]).await

        }).await
    }
        }
    };
}
