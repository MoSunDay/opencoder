use opencoder_core::brain::*;
use std::collections::BTreeSet;

/// Inherit only shared ancestry. Consuming different arms of a split closes
/// that scope; a subsequent split gets a fresh durable route receipt identity.
pub(super) fn child_scope(run: &BrainRun, parents: &[String]) -> Vec<GraphScope> {
    let Some(first) = parents.first() else {
        return vec![];
    };
    let mut scope = run.graph.visits[first].scope.clone();
    for parent in &parents[1..] {
        let other = &run.graph.visits[parent].scope;
        let shared = scope.iter().zip(other).take_while(|(a, b)| a == b).count();
        scope.truncate(shared);
    }
    scope
}

/// Pair join inputs within the nearest active split. Separate invocations of a
/// nested parallel region never share a cohort, even when output names match.
pub(super) fn cohorts(
    run: &BrainRun,
    relevant: &[String],
    producers: &BTreeSet<&String>,
) -> Vec<Vec<String>> {
    let mut groups = BTreeSet::new();
    for id in relevant {
        if !producers.contains(&run.instances[id].step_id) {
            continue;
        }
        let token = &run.graph.tokens[id];
        let mut group = None;
        for depth in (0..token.scope.len()).rev() {
            let candidates: Vec<_> = relevant
                .iter()
                .filter(|other| {
                    let scope = &run.graph.tokens[*other].scope;
                    scope.len() > depth
                        && scope[..depth] == token.scope[..depth]
                        && scope[depth].fork == token.scope[depth].fork
                })
                .cloned()
                .collect();
            let arms: BTreeSet<_> = candidates
                .iter()
                .map(|other| &run.graph.tokens[other].scope[depth].branch)
                .collect();
            if arms.len() > 1 {
                group = Some(candidates);
                break;
            }
        }
        groups.insert(group.unwrap_or_else(|| {
            relevant
                .iter()
                .filter(|other| run.graph.tokens[*other].scope == token.scope)
                .cloned()
                .collect()
        }));
    }
    let mut groups: Vec<Vec<String>> = groups.into_iter().collect();
    groups.sort_by_key(Vec::len);
    // An inner join owns its tokens before an enclosing join may observe them.
    // Do not prepare overlapping receipts in a single durable state revision.
    let mut claimed = BTreeSet::new();
    groups
        .into_iter()
        .filter(|group| {
            if group.iter().any(|id| claimed.contains(id)) {
                return false;
            }
            claimed.extend(group.iter().cloned());
            true
        })
        .collect()
}
