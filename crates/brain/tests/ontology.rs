#[path = "graph/support.rs"]
mod support;
use opencoder_brain::ontology::validate;
use support::*;
#[test]
fn graph_validator_accepts_loop_and_rejects_legacy_invalid_ports_and_dead_ends() {
    assert!(validate(&fixture()).is_ok());
    let mut p = fixture();
    p.schema_version = 1;
    assert!(validate(&p)
        .unwrap_err()
        .to_string()
        .contains("migration required"));
    let mut p = fixture();
    p.routes[0].targets[0].instance = "unknown".into();
    assert!(validate(&p).is_err());
    let mut p = fixture();
    p.routes[0].targets[0].bindings.clear();
    assert!(validate(&p).is_err());
    let mut p = fixture();
    p.routes[1].exits.clear();
    assert!(validate(&p).is_err());
    let mut p = fixture();
    p.outputs
        .get_mut("verification")
        .unwrap()
        .description
        .clear();
    assert!(validate(&p).is_err());
    let mut p = fixture();
    p.routes[0].outputs = vec!["foreign".into()];
    assert!(validate(&p).is_err());
}
