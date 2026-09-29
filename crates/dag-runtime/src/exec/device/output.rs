//! Validate model output against authoritative device allocation before expansion.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) fn reservation_id(dag_id: &str) -> String {
    let key = serde_json::to_vec(&[dag_id, "execute"]).expect("string array JSON");
    format!("{:x}", Sha256::digest(key))[..32].into()
}

pub(super) fn ui_run_id(reservation: &str, instance: &str, case: &str) -> String {
    let bytes = serde_json::to_vec(&[reservation, instance, case]).expect("string array JSON");
    format!("ui-{}", &format!("{:x}", Sha256::digest(bytes))[..40])
}

pub(super) fn validate_ui(
    output: &Value,
    reservation: &Value,
    input: &Value,
    dag: &str,
) -> Result<()> {
    let frozen = opencoder_dag::ui_cases::public_input(input).map_err(anyhow::Error::msg)?;
    let count = frozen["device_count"].as_u64().unwrap() as usize;
    let id = reservation_id(dag);
    ensure!(
        reservation["reservation_id"] == id
            && reservation["dag_id"] == dag
            && reservation["target_step"] == "execute"
            && reservation["count"] == count
            && reservation["work_type"] == "ui",
        "UI allocation authority mismatch"
    );
    let rows = reservation["assignments"]
        .as_array()
        .context("UI assignments missing")?;
    let items = output["items"]
        .as_array()
        .context("UI allocator items missing")?;
    ensure!(
        rows.len() == count && items.len() == count,
        "UI allocator omitted a device"
    );
    let mut machines = std::collections::BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let instance_id = index.to_string();
        let found: Vec<_> = rows
            .iter()
            .filter(|r| r["instance_id"] == instance_id)
            .collect();
        ensure!(found.len() == 1, "UI assignment missing or duplicated");
        let row = found[0];
        let machine = row["machine"].as_str().context("UI machine missing")?;
        ensure!(
            super::valid_machine(machine) && machines.insert(machine),
            "UI machine invalid or duplicated"
        );
        ensure!(
            row["generation"].as_u64().is_some(),
            "UI generation missing"
        );
        let cases =
            opencoder_dag::ui_cases::case_batch(input, index).map_err(anyhow::Error::msg)?;
        let ids: Vec<_> = cases.iter().map(|c| c["case_id"].clone()).collect();
        ensure!(
            row["case_ids"] == json!(ids),
            "UI reservation case ownership changed"
        );
        let value: Value =
            serde_json::from_str(item.as_str().context("UI item must be JSON string")?)?;
        ensure!(
            value
                == json!({"instance_id":index.to_string(),"machine":machine,
            "generation":row["generation"],"reservation_id":id,"cases":cases}),
            "UI allocator output differs from host assignment"
        );
    }
    Ok(())
}

pub(super) fn completed_ui(reservation: &Value, input: &Value, identity: &Value) -> Result<()> {
    let dag = identity["dag_id"]
        .as_str()
        .context("UI DAG identity missing")?;
    let instance = identity["instance_id"]
        .as_str()
        .context("UI instance missing")?;
    let index: usize = instance.parse()?;
    let id = reservation_id(dag);
    ensure!(
        reservation["reservation_id"] == id
            && reservation["dag_id"] == dag
            && reservation["work_type"] == "ui",
        "UI completion authority mismatch"
    );
    let rows: Vec<_> = reservation["assignments"]
        .as_array()
        .context("UI assignments missing")?
        .iter()
        .filter(|r| r["instance_id"] == instance)
        .collect();
    ensure!(rows.len() == 1, "UI completion assignment missing");
    let row = rows[0];
    ensure!(
        row["session_id"] == identity["session_id"] && row["released"] != true,
        "UI session no longer owns assignment"
    );
    let cases = opencoder_dag::ui_cases::case_batch(input, index).map_err(anyhow::Error::msg)?;
    let expected: Vec<_> = cases
        .iter()
        .map(|c| ui_run_id(&id, instance, c["case_id"].as_str().unwrap()))
        .collect();
    let works = row["works"]
        .as_array()
        .context("UI work receipts missing")?;
    let actual: Vec<_> = works
        .iter()
        .map(|w| w["ui_run_id"].as_str().unwrap_or("").to_string())
        .collect();
    ensure!(
        actual == expected && works.iter().all(|w| w["recovery_verified"] == true),
        "UI cases missing, foreign, or not restored"
    );
    Ok(())
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    #[test]
    fn ui_assignment_and_completion_require_exact_frozen_cases() {
        let input = json!({"device_count":1,"case_source":"/private/specs",
            "cases":[{"case_id":"BITS-1-a","spec_sha256":"a".repeat(64),"input_version":"v1"}]});
        let dag = "dag-ui";
        let id = reservation_id(dag);
        let cases = opencoder_dag::ui_cases::case_batch(&input, 0).unwrap();
        let run = ui_run_id(&id, "0", "BITS-1-a");
        let mut reservation = json!({"reservation_id":id,"dag_id":dag,"target_step":"execute",
            "count":1,"work_type":"ui","assignments":[{"instance_id":"0","machine":"win-02",
            "generation":1,"case_ids":["BITS-1-a"],"session_id":"host","works":[{"ui_run_id":run,
            "recovery_verified":true}]}]});
        let item = json!({"instance_id":"0","machine":"win-02","generation":1,
            "reservation_id":id,"cases":cases});
        assert!(validate_ui(
            &json!({"items":[item.to_string()]}),
            &reservation,
            &input,
            dag
        )
        .is_ok());
        let identity = json!({"dag_id":dag,"instance_id":"0","session_id":"host"});
        assert!(completed_ui(&reservation, &input, &identity).is_ok());
        reservation["assignments"][0]["works"][0]["recovery_verified"] = json!(false);
        assert!(completed_ui(&reservation, &input, &identity).is_err());
        reservation["assignments"][0]["case_ids"] = json!(["foreign"]);
        assert!(validate_ui(
            &json!({"items":[item.to_string()]}),
            &reservation,
            &input,
            dag
        )
        .is_err());
    }
}

pub(super) fn validate(
    output: &Value,
    reservation: &Value,
    input: &Value,
    dag: &str,
) -> Result<()> {
    let frozen = opencoder_dag::devices::public_input(input).map_err(anyhow::Error::msg)?;
    let count = frozen["device_count"].as_u64().unwrap() as usize;
    let id = reservation_id(dag);
    ensure!(
        reservation["reservation_id"] == id
            && reservation["dag_id"] == dag
            && reservation["target_step"] == "execute"
            && reservation["count"] == count,
        "Allocation authority identity/count mismatch"
    );
    let assigned = reservation["assignments"]
        .as_array()
        .context("Allocation assignments missing")?;
    let items = output["items"]
        .as_array()
        .context("Allocator output items missing")?;
    ensure!(
        assigned.len() == count && items.len() == count,
        "Allocator output must contain every requested device exactly once"
    );
    ensure!(
        reservation.get("eligible_machines") == frozen.get("eligible_machines"),
        "Allocator eligible machine scope changed"
    );
    let mut machines = std::collections::BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let instance_id = index.to_string();
        let matches: Vec<_> = assigned
            .iter()
            .filter(|a| a["instance_id"] == instance_id)
            .collect();
        ensure!(
            matches.len() == 1,
            "Authority instance missing or duplicated"
        );
        let row = matches[0];
        let machine = row["machine"]
            .as_str()
            .context("Authority machine missing")?;
        ensure!(
            super::valid_machine(machine) && machines.insert(machine),
            "Authority machine invalid or duplicated"
        );
        if let Some(eligible) = frozen.get("eligible_machines").and_then(Value::as_array) {
            ensure!(
                eligible.iter().any(|item| item.as_str() == Some(machine)),
                "Authority machine outside eligible scope"
            );
        }
        ensure!(
            row["generation"].as_u64().is_some(),
            "Authority generation missing"
        );
        let value: Value = serde_json::from_str(
            item.as_str()
                .context("Allocator item must be a JSON string")?,
        )?;
        let cases = opencoder_dag::devices::case_batch(input, index).map_err(anyhow::Error::msg)?;
        ensure!(
            value
                == json!({"instance_id":index.to_string(),"machine":machine,"generation":row["generation"],"reservation_id":id,"case_ids":cases}),
            "Allocator output differs from authoritative device assignment"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_cannot_drop_duplicate_add_or_redirect_assignments() {
        let input = json!({"device_count":2,"case_ids":["a","b","c"]});
        let id = reservation_id("dag-test");
        let reservation = json!({"reservation_id":id,"dag_id":"dag-test","target_step":"execute","count":2,"assignments":[
            {"instance_id":"0","machine":"win-02","generation":1}, {"instance_id":"1","machine":"win-03","generation":4}]});
        let a = json!({"instance_id":"0","machine":"win-02","generation":1,"reservation_id":id,"case_ids":["a","c"]});
        let b = json!({"instance_id":"1","machine":"win-03","generation":4,"reservation_id":id,"case_ids":["b"]});
        let good = json!({"items":[a.to_string(),b.to_string()]});
        assert!(validate(&good, &reservation, &input, "dag-test").is_ok());
        for bad in [
            json!({"items":[]}),
            json!({"items":[a.to_string()]}),
            json!({"items":[a.to_string(),a.to_string()]}),
            json!({"items":[a.to_string(),b.to_string(),b.to_string()]}),
            json!({"items":[b.to_string(),a.to_string()]}),
        ] {
            assert!(validate(&bad, &reservation, &input, "dag-test").is_err());
        }
        for (key, value) in [
            ("reservation_id", json!("foreign")),
            ("machine", json!("win-04")),
            ("generation", json!(99)),
            ("case_ids", json!(["a"])),
        ] {
            let mut wrong = a.clone();
            wrong[key] = value;
            assert!(validate(
                &json!({"items":[wrong.to_string(),b.to_string()]}),
                &reservation,
                &input,
                "dag-test"
            )
            .is_err());
        }
    }

    #[test]
    fn accepts_admitted_windows_beyond_initial_fleet() {
        assert!(super::super::valid_machine("win-20"));
        assert!(super::super::valid_machine("win-31"));
        assert!(super::super::valid_machine("win-41"));
        for machine in ["win-01", "win-002", "win-20x", "win-00", "linux-20"] {
            assert!(!super::super::valid_machine(machine));
        }
    }

    #[test]
    fn allocation_stays_within_frozen_eligible_machines() {
        let input = json!({"device_count":1,"case_ids":["a"],"eligible_machines":["win-21"]});
        let id = reservation_id("dag-test");
        let assignment =
            |machine: &str| json!({"instance_id":"0","machine":machine,"generation":1});
        let item = |machine: &str| {
            json!({"instance_id":"0","machine":machine,
            "generation":1,"reservation_id":id,"case_ids":["a"]})
            .to_string()
        };
        let reservation = |machine: &str, scope: Value| {
            json!({"reservation_id":id,
            "dag_id":"dag-test","target_step":"execute","count":1,
            "eligible_machines":scope,"assignments":[assignment(machine)]})
        };
        assert!(validate(
            &json!({"items":[item("win-21")]}),
            &reservation("win-21", json!(["win-21"])),
            &input,
            "dag-test"
        )
        .is_ok());
        assert!(validate(
            &json!({"items":[item("win-22")]}),
            &reservation("win-22", json!(["win-21"])),
            &input,
            "dag-test"
        )
        .is_err());
        assert!(validate(
            &json!({"items":[item("win-21")]}),
            &reservation("win-21", json!(["win-22"])),
            &input,
            "dag-test"
        )
        .is_err());
    }
}

pub(super) fn completed(reservation: &Value, input: &Value, identity: &Value) -> Result<()> {
    let dag = identity["dag_id"]
        .as_str()
        .context("Host DAG identity missing")?;
    let instance = identity["instance_id"]
        .as_str()
        .context("Host instance missing")?;
    let index: usize = instance.parse()?;
    let id = reservation_id(dag);
    ensure!(
        reservation["reservation_id"] == id
            && reservation["dag_id"] == dag
            && reservation["target_step"] == "execute",
        "Completion authority identity mismatch"
    );
    let rows: Vec<_> = reservation["assignments"]
        .as_array()
        .context("Completion assignments missing")?
        .iter()
        .filter(|a| a["instance_id"] == instance)
        .collect();
    ensure!(rows.len() == 1, "Completion instance missing or duplicated");
    let row = rows[0];
    ensure!(
        row["session_id"] == identity["session_id"] && row["released"] != true,
        "Completion session no longer owns assignment"
    );
    let cases = opencoder_dag::devices::case_batch(input, index).map_err(anyhow::Error::msg)?;
    let expected: Vec<_> = cases
        .iter()
        .map(|case| {
            let bytes = serde_json::to_vec(&[id.as_str(), instance, case.as_str()])
                .expect("string array JSON");
            format!("nh-{}", &format!("{:x}", Sha256::digest(bytes))[..40])
        })
        .collect();
    let works = row["works"].as_array().context("Completed works missing")?;
    ensure!(
        works.len() == expected.len(),
        "Not every assigned case completed exactly once"
    );
    let actual: Vec<_> = works
        .iter()
        .map(|w| w["native_run_id"].as_str().unwrap_or("").to_string())
        .collect();
    ensure!(
        actual == expected && works.iter().all(|w| w["recovery_verified"] == true),
        "Assigned cases missing, foreign or not recovered"
    );
    Ok(())
}

#[cfg(test)]
mod completion_tests {
    use super::*;
    #[test]
    fn omitted_foreign_duplicate_and_unrecovered_cases_cannot_finish_successfully() {
        let id = reservation_id("dag-test");
        let input = json!({"device_count":1,"case_ids":["a"]});
        let identity = json!({"dag_id":"dag-test","instance_id":"0","session_id":"host"});
        let bytes = serde_json::to_vec(&[id.as_str(), "0", "a"]).unwrap();
        let native = format!("nh-{}", &format!("{:x}", Sha256::digest(bytes))[..40]);
        let mut row = json!({"instance_id":"0","session_id":"host","works":[{"native_run_id":native,"recovery_verified":true}]});
        let reservation = |r: Value| json!({"reservation_id":id,"dag_id":"dag-test","target_step":"execute","assignments":[r]});
        assert!(completed(&reservation(row.clone()), &input, &identity).is_ok());
        for works in [
            json!([]),
            json!([{"native_run_id":"foreign","recovery_verified":true}]),
            json!([{"native_run_id":native,"recovery_verified":false}]),
            json!([{"native_run_id":native,"recovery_verified":true},{"native_run_id":native,"recovery_verified":true}]),
        ] {
            row["works"] = works;
            assert!(completed(&reservation(row.clone()), &input, &identity).is_err());
        }
    }
    #[test]
    fn completion_preserves_frozen_case_order_and_does_not_infer_business_success() {
        let id = reservation_id("dag-test");
        let input = json!({"device_count":1,"case_ids":["a","b"]});
        let identity = json!({"dag_id":"dag-test","instance_id":"0","session_id":"host"});
        let works:Vec<_>=["a","b"].into_iter().map(|case| {
            let bytes=serde_json::to_vec(&[id.as_str(),"0",case]).unwrap();
            json!({"native_run_id":format!("nh-{}",&format!("{:x}",Sha256::digest(bytes))[..40]),"recovery_verified":true,"business_passed":false})
        }).collect();
        let mut reservation = json!({"reservation_id":id,"dag_id":"dag-test","target_step":"execute","assignments":[{"instance_id":"0","session_id":"host","works":works}]});
        assert!(completed(&reservation, &input, &identity).is_ok());
        reservation["assignments"][0]["works"]
            .as_array_mut()
            .unwrap()
            .reverse();
        assert!(completed(&reservation, &input, &identity).is_err());
    }
}
