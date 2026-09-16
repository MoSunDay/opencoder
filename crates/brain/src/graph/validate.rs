use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::*, fleet::valid_id};
use std::collections::{BTreeMap, BTreeSet};

pub fn validate(plan: &OntologyPlan) -> Result<()> {
    ensure!(plan.schema_version == 2, "{}", super::MIGRATION);
    ensure!(
        plan.steps.is_empty() && plan.flow.is_none() && plan.deliverables.is_empty(),
        "v2 uses instances, outputs and routes; legacy scheduling fields are forbidden"
    );
    ensure!(
        !plan.title.trim().is_empty() && !plan.objective.trim().is_empty(),
        "plan needs title and objective"
    );
    ensure!(
        (1..=200).contains(&plan.instances.len()),
        "plan needs 1..200 instances"
    );
    ensure!(
        (1..=1000).contains(&plan.routes.len()),
        "plan needs 1..1000 routes"
    );
    let instances: BTreeMap<_, _> = plan.instances.iter().map(|i| (i.id.as_str(), i)).collect();
    ensure!(
        instances.len() == plan.instances.len(),
        "duplicate instance id"
    );
    let mut owners = BTreeMap::new();
    for instance in &plan.instances {
        ensure!(valid_id(&instance.id), "invalid instance id");
        ensure!(
            !instance.description.trim().is_empty() && !instance.capability_id.trim().is_empty(),
            "instance {} requires description and registered capability_id",
            instance.id
        );
        ensure!(
            BUSINESS_KINDS.contains(&instance.action.kind),
            "unsupported capability kind"
        );
        ensure!(
            !instance.action.target.trim().is_empty() && !instance.action.prompt.trim().is_empty(),
            "capability target and prompt required"
        );
        ensure!(
            (1..=100).contains(&instance.max_visits),
            "max_visits must be 1..100"
        );
        ensure!(
            (1..=10).contains(&instance.action.max_attempts),
            "max_attempts must be 1..10"
        );
        ensure!(
            !instance.inputs.is_empty() && !instance.outputs.is_empty(),
            "instance {} requires inputs and outputs",
            instance.id
        );
        ensure!(
            unique(&instance.inputs) && unique(&instance.outputs),
            "duplicate instance ports"
        );
        for input in &instance.inputs {
            ensure!(plan.inputs.contains_key(input), "undeclared input {input}");
        }
        for output in &instance.outputs {
            ensure!(
                plan.outputs.contains_key(output),
                "undeclared output {output}"
            );
            ensure!(
                owners
                    .insert(output.as_str(), instance.id.as_str())
                    .is_none(),
                "output {output} has multiple owners"
            );
        }
        let keys: BTreeSet<_> = instance.resources.iter().map(|r| &r.key).collect();
        ensure!(
            keys.len() == instance.resources.len() && keys.iter().all(|k| !k.trim().is_empty()),
            "invalid resources"
        );
    }
    ensure!(owners.len() == plan.outputs.len(), "unowned output");
    for (id, output) in &plan.outputs {
        ensure!(
            valid_id(id) && !output.description.trim().is_empty(),
            "output {id} needs a name and description"
        );
    }
    for (id, input) in &plan.inputs {
        ensure!(
            valid_id(id) && !input.description.trim().is_empty(),
            "input {id} needs a name and description"
        );
        crate::ontology::validate_schema(&input.schema, 0)?;
        ensure!(
            plan.instances.iter().any(|i| i.inputs.contains(id)),
            "unused input {id}"
        );
    }
    ensure!(
        !plan.entry.is_empty() && unique(&plan.entry),
        "declare unique entry instances"
    );
    for entry in &plan.entry {
        let instance = instances.get(entry.as_str()).context("unknown entry")?;
        ensure!(instance.inputs.iter().all(|i| !plan.inputs[i].required || plan.inputs[i].source == InputSource::External), "entry requires routed input");
    }
    let mut route_ids = BTreeSet::new();
    let mut outgoing = BTreeMap::new();
    for route in &plan.routes {
        ensure!(
            valid_id(&route.id) && route_ids.insert(&route.id),
            "invalid or duplicate route id"
        );
        ensure!(
            !route.description.trim().is_empty()
                && !route.outputs.is_empty()
                && unique(&route.outputs),
            "route requires semantics and distinct outputs"
        );
        ensure!(
            !route.targets.is_empty() || !route.exits.is_empty(),
            "route needs adjacent targets or exit"
        );
        for output in &route.outputs {
            let owner = owners
                .get(output.as_str())
                .context("unknown route output")?;
            if let Some(previous) = outgoing.insert(*owner, route.id.as_str()) {
                ensure!(
                    previous == route.id,
                    "all outputs of an instance must share one routing decision"
                );
            }
        }
        let mut targets = BTreeSet::new();
        for target in &route.targets {
            ensure!(targets.insert(&target.instance), "duplicate route target");
            let instance = instances
                .get(target.instance.as_str())
                .context("nonexistent route target")?;
            for (input, output) in &target.bindings {
                ensure!(
                    instance.inputs.contains(input),
                    "binding targets foreign input {input}"
                );
                ensure!(
                    plan.inputs[input].source == InputSource::Routed,
                    "binding cannot replace external input"
                );
                ensure!(
                    route.outputs.contains(output),
                    "binding uses unconnected output"
                );
            }
            for input in &instance.inputs {
                ensure!(
                    !plan.inputs[input].required
                        || plan.inputs[input].source == InputSource::External
                        || target.bindings.contains_key(input),
                    "missing mapping for required input {input}"
                );
            }
        }
        let mut exits = BTreeSet::new();
        for exit in &route.exits {
            ensure!(
                valid_id(&exit.id) && exits.insert(&exit.id) && !exit.description.trim().is_empty(),
                "invalid exit"
            );
            ensure!(
                !exit.deliverables.is_empty() && unique(&exit.deliverables),
                "exit must declare deliverables"
            );
            ensure!(
                exit.deliverables.iter().all(|o| route.outputs.contains(o)),
                "exit reads unconnected output"
            );
        }
    }
    ensure!(
        outgoing.len() == instances.len(),
        "every instance needs an output route"
    );
    let mut reachable: BTreeSet<_> = plan.entry.iter().map(String::as_str).collect();
    let mut finishing = BTreeSet::new();
    loop {
        let before = (reachable.len(), finishing.len());
        for route in &plan.routes {
            let sources: Vec<_> = route.outputs.iter().map(|o| owners[o.as_str()]).collect();
            if sources.iter().any(|s| reachable.contains(s)) {
                reachable.extend(route.targets.iter().map(|t| t.instance.as_str()));
            }
            if !route.exits.is_empty()
                || route
                    .targets
                    .iter()
                    .any(|t| finishing.contains(t.instance.as_str()))
            {
                finishing.extend(sources);
            }
        }
        if before == (reachable.len(), finishing.len()) {
            break;
        }
    }
    ensure!(reachable.len() == instances.len(), "unreachable instance");
    ensure!(
        finishing.len() == instances.len(),
        "every instance needs a structural path to an exit"
    );
    Ok(())
}
fn unique(values: &[String]) -> bool {
    values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
