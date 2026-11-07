use anyhow::{Context, Result};

use turso::Connection;

use super::chat_tables::{
    CREATE_EVENTS, CREATE_INPUTS, CREATE_MESSAGES, CREATE_SESSIONS, CREATE_SUBAGENT_TASKS,
};

use super::schedule::{CREATE_INDEX_SCHEDULE_RUNS_FIRED, CREATE_SCHEDULES, CREATE_SCHEDULE_RUNS};

use super::team_runs::{CREATE_INDEX_TEAM_TOPIC_RUNS_TOPIC, CREATE_TEAM_TOPIC_RUNS};

#[path = "schema/access.rs"]
mod access;

#[path = "schema/catalog.rs"]
mod catalog;

#[path = "schema/migrations.rs"]
mod migrations;

#[path = "schema/project_links.rs"]
mod project_links;

#[path = "schema/project_people.rs"]
mod project_people;

#[path = "schema/project_relations.rs"]
mod project_relations;

use migrations::migrate;

use project_relations::CREATE_PROJECT_MILESTONES;
include!("schema/parts/part_1.rs");

include!("schema/parts/part_2.rs");

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
include!("schema/parts/part_3.rs");
