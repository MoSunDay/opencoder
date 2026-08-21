//! `opencode client session|questions` subcommand implementations: management
//! operations over the remote server (list/show/delete/fork sessions, list/
//! answer/skip pending question cards). Pure request→print adapters: no local
//! state, no LLM.

use anyhow::Result;
use opencoder_client::Remote;

use crate::{ClientQuestionsSub, ClientSessionSub, ClientSub};

/// Dispatch a `client` management subcommand. `workdir` is used by `session
/// list` to filter remote sessions (defaults to cwd, see
/// [`crate::client::resolve_client_workdir`]).
pub(crate) async fn client_dispatch_sub(
    remote: &Remote,
    sub: ClientSub,
    workdir: &str,
) -> Result<()> {
    match sub {
        ClientSub::Session { sub } => client_session_sub(remote, sub, workdir).await,
        ClientSub::Questions { sub } => client_questions_sub(remote, sub).await,
    }
}

/// `opencode client session <list|show|delete|fork>`.
pub async fn client_session_sub(
    remote: &Remote,
    sub: ClientSessionSub,
    workdir: &str,
) -> Result<()> {
    match sub {
        ClientSessionSub::List => {
            let items = remote.list_sessions(Some(50), None, Some(workdir)).await?;
            if items.is_empty() {
                println!("(no sessions for workdir {workdir}; pass a different --workdir)");
                return Ok(());
            }
            for it in items {
                let id = it.get("id").and_then(|v| v.as_str()).unwrap_or("(?)");
                let title = it
                    .get("title")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("(untitled)");
                let preview = it.get("preview").and_then(|v| v.as_str()).unwrap_or("");
                println!("{id}\t{title}\t{preview}");
            }
            Ok(())
        }
        ClientSessionSub::Show { id } => {
            let v = remote.get_session(&id).await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
            Ok(())
        }
        ClientSessionSub::Delete { id } => {
            remote.delete_session(&id).await?;
            println!("deleted {id}");
            Ok(())
        }
        ClientSessionSub::Fork { id } => {
            let new_id = remote.fork_session(&id).await?;
            println!("{new_id}");
            Ok(())
        }
    }
}

/// `opencode client questions <list|answer|skip>`.
pub async fn client_questions_sub(remote: &Remote, sub: ClientQuestionsSub) -> Result<()> {
    match sub {
        ClientQuestionsSub::List { session } => {
            let questions = remote.list_questions(&session).await?;
            if questions.is_empty() {
                println!("(no questions waiting)");
                return Ok(());
            }
            for q in questions {
                let call_id = q.get("id").and_then(|v| v.as_str()).unwrap_or("(?)");
                let text = q.get("question").and_then(|v| v.as_str()).unwrap_or("");
                let options: Vec<&str> = q
                    .get("options")
                    .and_then(|o| o.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                let opts = if options.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", options.join("|"))
                };
                println!("{call_id}\t{text}{opts}");
            }
            Ok(())
        }
        ClientQuestionsSub::Answer {
            session,
            call_id,
            answer,
        } => {
            remote.answer_question(&session, &call_id, &answer).await?;
            println!("answered {call_id}");
            Ok(())
        }
        ClientQuestionsSub::Skip { session, call_id } => {
            remote.skip_question(&session, &call_id).await?;
            println!("skipped {call_id}");
            Ok(())
        }
    }
}
