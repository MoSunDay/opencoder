use super::derive::remove_member;
use super::*;
use opencoder_core::fleet::ExecutionKind;
use std::time::{Duration, Instant};

impl InventoryCache {
    pub(super) async fn refresh(&mut self, worker: &Worker) -> Result<()> {
        let started = Instant::now();
        let reconcile = self
            .checked
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(600));
        // Finish fallible reads before advancing any cached source watermark.
        let source = worker
            .inner
            .state
            .store
            .inventory_sources(if reconcile { None } else { self.db_revision })
            .await?;
        let (journal, targets) = {
            let journal = worker.inner.journal.lock().await;
            let changes = journal.inventory.read(if reconcile {
                None
            } else {
                self.journal_revision
            });
            let targets: BTreeMap<_, _> = changes
                .records
                .iter()
                .filter(|r| r.kind == ExecutionKind::Project)
                .filter_map(|r| {
                    journal
                        .summaries
                        .get(&r.id)?
                        .target
                        .as_ref()
                        .map(|t| (r.id.clone(), t.clone()))
                })
                .collect();
            (changes, targets)
        };
        let active_loops: BTreeSet<_> = opencoder_session::loop_registry::active_ids()
            .into_iter()
            .collect();
        let active_roots: BTreeSet<_> = worker.inner.active.lock().await.keys().cloned().collect();
        let source_rows = source.sessions.len() + source.project_runs.len() + journal.records.len();
        let full_read = source.full || journal.full;
        let mut dirty = BTreeSet::new();
        let mut dirty_todos = BTreeSet::new();
        let mut running_todos = BTreeSet::new();
        dirty.extend(
            self.active_loops
                .symmetric_difference(&active_loops)
                .cloned(),
        );
        for id in self.active_roots.symmetric_difference(&active_roots) {
            if let Some(todo) = self.targets.get(id) {
                running_todos.insert(todo.clone());
            }
        }
        self.active_loops = active_loops;
        self.active_roots = active_roots;
        if source.full {
            dirty.extend(self.sessions.keys().cloned());
            dirty.extend(self.runs.keys().cloned());
            self.sessions.clear();
            self.runs.clear();
            self.runs_by_todo.clear();
            self.running_by_todo.clear();
        }
        for id in source.removed_sessions {
            self.sessions.remove(&id);
            dirty.insert(id);
        }
        for id in source.removed_runs {
            self.remove_run(&id);
            dirty.insert(id);
        }
        for session in source.sessions {
            dirty.insert(session.id.clone());
            self.sessions.insert(session.id.clone(), session);
        }
        for run in source.project_runs {
            dirty.insert(run.id.clone());
            self.put_run(run);
        }
        if journal.full {
            dirty.extend(self.roots.keys().cloned());
            dirty_todos.extend(self.roots_by_todo.keys().cloned());
            self.roots.clear();
            self.targets.clear();
            self.roots_by_todo.clear();
        }
        for id in journal.removed_ids {
            self.roots.remove(&id);
            if let Some(todo) = self.targets.remove(&id) {
                remove_member(&mut self.roots_by_todo, &todo, &id);
                dirty_todos.insert(todo);
            }
            dirty.insert(id);
        }
        for root in journal.records {
            let old = self.targets.get(&root.id);
            let new = targets.get(&root.id);
            if old != new {
                if let Some(todo) = old.cloned() {
                    remove_member(&mut self.roots_by_todo, &todo, &root.id);
                    dirty_todos.insert(todo);
                }
                self.targets.remove(&root.id);
                if let Some(todo) = new {
                    self.targets.insert(root.id.clone(), todo.clone());
                    self.roots_by_todo
                        .entry(todo.clone())
                        .or_default()
                        .insert(root.id.clone());
                    dirty_todos.insert(todo.clone());
                }
            } else if let Some(todo) = new {
                running_todos.insert(todo.clone());
            }
            dirty.insert(root.id.clone());
            self.roots.insert(root.id.clone(), root);
        }
        for todo in dirty_todos {
            if let Some(ids) = self.runs_by_todo.get(&todo) {
                dirty.extend(ids.iter().cloned());
            }
        }
        for todo in running_todos {
            if let Some(ids) = self.running_by_todo.get(&todo) {
                dirty.extend(ids.iter().cloned());
            }
        }
        let affected = dirty.len();
        for id in dirty {
            if let Some(index) = self.index(&id, &worker.inner.registration.id) {
                self.log.set(index);
            } else {
                self.log.remove(&id);
            }
        }
        self.db_revision = Some(source.revision);
        self.journal_revision = Some(journal.revision);
        if reconcile {
            self.checked = Some(Instant::now());
        }
        self.metrics
            .observe(source_rows, affected, full_read, started.elapsed());
        tracing::debug!(
            source_rows,
            affected,
            reconcile,
            elapsed_us = started.elapsed().as_micros() as u64,
            "runtime inventory refresh"
        );
        Ok(())
    }
}
