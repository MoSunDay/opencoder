//! P1 contract tests for skill discovery: the public API surface
//! (`discover`, `parse_skill`, `skills_dir`, `Skill` fields), file-layout
//! handling (flat `.md` vs nested `SKILL.md`), frontmatter parsing, and the
//! "missing directory is not an error" guarantee the TUI picker relies on.

use std::fs;
use std::sync::Mutex;

use opencoder_core::skill::{
    discover_in, parse_skill, seed_builtin_skills_in, seed_dep_gated_skills_in,
};
use opencoder_core::{discover_skills, skills_dir, Skill, DEPS_SENTINEL};

// Env mutation is process-global; serialize the HOME-manipulating tests.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn write(path: &std::path::Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

mod discovery;
mod planning;
mod seeding;
mod workflows;
