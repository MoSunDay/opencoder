//! Editable TODO notes, distinct from task acceptance and execution results.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewComment {
    pub todo_id: String,
    pub request_id: String,
    pub body_md: String,
    pub author_kind: String,
    pub author_id: String,
    pub author_name: String,
    pub capability_kind: Option<String>,
    pub submitted_by: String,
    pub execution_id: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub seq: i64,
    pub revision: i64,
    pub updated_at: i64,
    #[serde(skip)]
    pub original_body_md: Option<String>,
    #[serde(flatten)]
    pub entry: NewComment,
}

impl Comment {
    /// Registry names and timestamps may change between retries; input cannot.
    pub fn matches_input(
        &self,
        body: &str,
        capability: Option<&str>,
        execution: Option<&str>,
    ) -> bool {
        self.original_body_md
            .as_deref()
            .unwrap_or(&self.entry.body_md)
            == body
            && (self.entry.author_kind == "capability").then_some(self.entry.author_id.as_str())
                == capability
            && self.entry.execution_id.as_deref() == execution
    }
}

#[derive(Debug, Clone)]
pub struct CommentMutation {
    pub todo_id: String,
    pub seq: i64,
    pub expected_revision: i64,
    /// None deletes the comment; Some replaces only its Markdown body.
    pub body_md: Option<String>,
    pub actor: opencoder_core::identity::Identity,
    pub now: i64,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommentQuery {
    pub before_seq: Option<i64>,
    pub after_seq: Option<i64>,
    pub limit: Option<usize>,
}

impl CommentQuery {
    pub fn valid(self) -> bool {
        !(self.before_seq.is_some() && self.after_seq.is_some())
            && self.before_seq.is_none_or(|v| v > 0)
            && self.after_seq.is_none_or(|v| v > 0)
            && self.limit.is_none_or(|v| (1..=100).contains(&v))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommentPage {
    pub comments: Vec<Comment>,
    pub has_more: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CommentError {
    #[error("TODO 不存在")]
    MissingTodo,
    #[error("评论不存在")]
    MissingComment,
    #[error("评论已删除，不能重放原追加请求")]
    Deleted,
    #[error("只能修改自己提交的评论；管理员可管理所有评论")]
    Forbidden,
    #[error("评论已变化，请重新读取后再操作")]
    RevisionConflict,
    #[error("评论内容或版本无效")]
    InvalidInput,
    #[error("相同 request_id 已用于其他评论内容")]
    Conflict,
    #[error("评论执行来源已解除关联或能力不一致，请重新选择")]
    InvalidSource,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_rejects_ambiguous_or_unbounded_queries() {
        assert!(CommentQuery::default().valid());
        for raw in [
            r#"{"before_seq":1,"after_seq":2}"#,
            r#"{"limit":101}"#,
            r#"{"after_seq":0}"#,
        ] {
            assert!(!serde_json::from_str::<CommentQuery>(raw).unwrap().valid());
        }
        assert!(
            serde_json::from_str::<CommentQuery>(r#"{"limit":100,"after_seq":1}"#)
                .unwrap()
                .valid()
        );
    }
}
