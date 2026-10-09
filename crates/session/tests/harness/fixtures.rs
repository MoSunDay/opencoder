use std::path::Path;

mod session;
pub use session::session;

pub fn card(root: &Path) {
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    for (cat, files) in [
        (
            "prompts",
            vec![
                ("soul.md", "SOUL_FIXTURE"),
                ("how.md", "HOW_FIXTURE"),
                ("output.md", "OUTPUT_FIXTURE"),
            ],
        ),
        (
            "skills",
            vec![(
                "inspect/SKILL.md",
                "---\nname: inspect\ndescription: Inspect\n---\nSKILL_FIXTURE",
            )],
        ),
        ("memory", vec![("memory.md", "MEMORY_FIXTURE")]),
        ("tools", vec![("probe", "#!/bin/sh\nprintf TOOL_OK")]),
    ] {
        let pool = root.join(cat).join("shared");
        let ver = pool.join("v1");
        std::fs::create_dir_all(&ver).unwrap();
        std::fs::write(
            pool.join("meta.json"),
            json!({"name":"shared","current":1,"history":[1]}).to_string(),
        )
        .unwrap();
        for (name, text) in files {
            let file = ver.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, text).unwrap();
            if cat == "tools" {
                std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
    }
    std::fs::create_dir_all(root.join("custom")).unwrap();
    std::fs::write(root.join("custom/meta.json"), json!({"name":"custom","harness":"codex","current":{"prompt":"shared","skills":"shared","tools":"shared","memory":"shared"}}).to_string()).unwrap();
}
