use super::schema::{accepts, compatible, projected, validate_schema};
use anyhow::{bail, ensure, Context, Result};
use opencoder_core::{brain::*, fleet::valid_id};
use std::collections::{BTreeMap, BTreeSet};

pub fn dependencies(step: &StepTemplate) -> BTreeSet<String> {
    let mut deps: BTreeSet<_> = step.depends_on.iter().cloned().collect();
    for binding in step
        .inputs
        .values()
        .map(|i| &i.binding)
        .chain(step.when.iter().map(|c| &c.value))
        .chain(step.expansion.iter().map(|e| &e.items))
    {
        if let Binding::Output { step, .. } = binding {
            deps.insert(step.clone());
        }
    }
    deps
}

pub fn validate(plan: &OntologyPlan) -> Result<()> {
    ensure!(
        plan.schema_version == 1,
        "unsupported ontology schema version"
    );
    ensure!(
        !plan.title.trim().is_empty() && !plan.objective.trim().is_empty(),
        "plan needs title and objective"
    );
    ensure!(
        !plan.steps.is_empty() && plan.steps.len() <= 200,
        "plan must contain 1..200 step templates"
    );
    ensure!(
        !plan.deliverables.is_empty(),
        "plan must declare deliverables"
    );
    let steps: BTreeMap<_, _> = plan.steps.iter().map(|s| (s.id.as_str(), s)).collect();
    ensure!(steps.len() == plan.steps.len(), "duplicate step ids");
    for (name, input) in &plan.inputs {
        ensure!(valid_id(name), "invalid input name {name}");
        validate_schema(&input.schema, 0).with_context(|| format!("input {name}"))?;
    }
    for step in &plan.steps {
        ensure!(valid_id(&step.id), "invalid step id {}", step.id);
        ensure!(
            BUSINESS_KINDS.contains(&step.action.kind),
            "{} is not a business capability",
            step.action.kind.prefix()
        );
        ensure!(
            !step.action.target.trim().is_empty() && !step.action.prompt.trim().is_empty(),
            "step {} needs target and action prompt",
            step.id
        );
        ensure!(
            !step.acceptance.trim().is_empty() && !step.purpose.trim().is_empty(),
            "step {} needs purpose and acceptance",
            step.id
        );
        ensure!(
            step.action.max_attempts > 0 && step.action.max_attempts <= 10,
            "max_attempts must be 1..10"
        );
        ensure!(
            step.resources.iter().all(|r| !r.key.trim().is_empty()),
            "empty resource key"
        );
        let resources: BTreeSet<_> = step.resources.iter().map(|r| &r.key).collect();
        ensure!(
            resources.len() == step.resources.len(),
            "duplicate resource declarations"
        );
        validate_schema(&step.output, 0)?;
        for dep in dependencies(step) {
            ensure!(
                steps.contains_key(dep.as_str()),
                "step {} references unknown dependency {dep}",
                step.id
            );
        }
        for (name, input) in &step.inputs {
            validate_schema(&input.schema, 0)?;
            check_binding(plan, Some(step), &input.binding, Some(&input.schema))
                .with_context(|| format!("step {} input {name}", step.id))?;
        }
        if let Some(condition) = &step.when {
            check_binding(plan, Some(step), &condition.value, None)?;
        }
        if let Some(expansion) = &step.expansion {
            ensure!(
                !matches!(expansion.items, Binding::Item { .. }),
                "foreach cannot depend on its own item"
            );
            let schema = binding_schema(plan, Some(step), &expansion.items)?;
            if let Some(schema) = schema {
                ensure!(
                    schema.kind == DataType::Array,
                    "foreach requires array input"
                );
                let key = projected(
                    schema.items.as_deref().context("missing items")?,
                    &expansion.key,
                )?;
                ensure!(
                    matches!(key.kind, DataType::String | DataType::Integer),
                    "foreach identity must be string or integer"
                );
            } else if let Binding::Literal { value } = &expansion.items {
                ensure!(value.is_array(), "foreach literal requires array");
            }
        }
    }
    if let Some(flow) = &plan.flow {
        super::flow::validate(plan, flow)?;
    } else {
        let mut resolved = BTreeSet::new();
        loop {
            let before = resolved.len();
            for step in &plan.steps {
                if dependencies(step).iter().all(|d| resolved.contains(d)) {
                    resolved.insert(step.id.clone());
                }
            }
            if resolved.len() == steps.len() {
                break;
            }
            ensure!(resolved.len() > before, "dependency cycle in ontology plan");
        }
    }
    for (name, deliverable) in &plan.deliverables {
        ensure!(valid_id(name), "invalid deliverable id");
        validate_schema(&deliverable.schema, 0)?;
        check_binding(plan, None, &deliverable.source, Some(&deliverable.schema))?;
        if let Some(expected) = &deliverable.expected {
            accepts(&deliverable.schema, expected)?;
        }
    }
    Ok(())
}

pub(super) fn check_condition(
    plan: &OntologyPlan,
    owner: &StepTemplate,
    condition: &Condition,
) -> Result<()> {
    check_binding(plan, Some(owner), &condition.value, None)?;
    if let Some(schema) = binding_schema(plan, Some(owner), &condition.value)? {
        accepts(&schema, &condition.equals)?;
    }
    Ok(())
}

pub(super) fn check_binding(
    plan: &OntologyPlan,
    owner: Option<&StepTemplate>,
    binding: &Binding,
    destination: Option<&DataSchema>,
) -> Result<()> {
    if let Binding::Literal { value } = binding {
        if let Some(schema) = destination {
            accepts(schema, value)?;
        }
        return Ok(());
    }
    let source = binding_schema(plan, owner, binding)?.context("unresolved binding schema")?;
    if let Some(destination) = destination {
        ensure!(
            compatible(&source, destination),
            "incompatible input/output or semantic types"
        );
    }
    Ok(())
}

fn binding_schema(
    plan: &OntologyPlan,
    owner: Option<&StepTemplate>,
    binding: &Binding,
) -> Result<Option<DataSchema>> {
    match binding {
        Binding::Literal { .. } => Ok(None),
        Binding::Input { name, path } => Ok(Some(
            projected(
                &plan
                    .inputs
                    .get(name)
                    .with_context(|| format!("undeclared input {name}"))?
                    .schema,
                path,
            )?
            .clone(),
        )),
        Binding::Output {
            step,
            path,
            collect,
        } => {
            let source = plan
                .steps
                .iter()
                .find(|s| s.id == *step)
                .with_context(|| format!("unknown output step {step}"))?;
            ensure!(
                source.expansion.is_none() || *collect,
                "expanded output {step} requires collect=true"
            );
            let mut schema = projected(&source.output, path)?.clone();
            if *collect {
                schema = DataSchema {
                    kind: DataType::Array,
                    semantic: None,
                    properties: BTreeMap::new(),
                    required: vec![],
                    items: Some(Box::new(schema)),
                    nullable: false,
                    values: vec![],
                };
            }
            Ok(Some(schema))
        }
        Binding::Item { path } => {
            let expansion = owner
                .and_then(|s| s.expansion.as_ref())
                .context("item binding outside foreach")?;
            if let Some(schema) = binding_schema(plan, None, &expansion.items)? {
                return Ok(Some(
                    projected(
                        schema.items.as_deref().context("foreach requires items")?,
                        path,
                    )?
                    .clone(),
                ));
            }
            bail!("literal foreach requires a typed plan input; bind the collection as input")
        }
    }
}
