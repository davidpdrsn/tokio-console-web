use anyhow::Context;
use axum_live_view::{html, Html};
use serde::{Deserialize, Serialize};

pub mod connection_state;
pub mod resources_index;
pub mod tasks_index;

mod layout;

pub use self::layout::{Layout, TaskResourceLayout};

#[derive(Deserialize, Serialize, Debug)]
struct Location {
    file: String,
    module_path: Option<String>,
    line: u32,
    column: u32,
}

impl Location {
    fn render<T>(&self) -> Html<T> {
        html! {
            { &self.file } ":" { self.line } ":" { self.column }
        }
    }
}

impl TryFrom<console_api::Location> for Location {
    type Error = anyhow::Error;

    fn try_from(location: console_api::Location) -> Result<Self, Self::Error> {
        Ok(Self {
            file: location
                .file
                .context("Missing `file` field")
                .map(truncate_registry_path)?,
            module_path: location.module_path,
            line: location.line.context("Missing `line` field")?,
            column: location.column.context("Missing `column` field")?,
        })
    }
}

fn truncate_registry_path(s: String) -> String {
    use once_cell::sync::OnceCell;
    use regex::Regex;
    use std::borrow::Cow;

    static REGEX: OnceCell<Regex> = OnceCell::new();
    let regex = REGEX.get_or_init(|| {
        Regex::new(r#".*/\.cargo(/registry/src/[^/]*/|/git/checkouts/)"#)
            .expect("failed to compile regex")
    });

    match regex.replace(&s, "{cargo}/") {
        Cow::Owned(s) => s,
        Cow::Borrowed(_) => s.to_string(),
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct MetaId(u64);

#[derive(Deserialize, Serialize, Debug)]
struct Metadata {
    id: MetaId,
    name: String,
    target: String,
}

impl TryFrom<console_api::register_metadata::NewMetadata> for Metadata {
    type Error = anyhow::Error;

    fn try_from(meta: console_api::register_metadata::NewMetadata) -> Result<Self, Self::Error> {
        let id = MetaId(meta.id.context("Missing `id` field")?.id);

        let meta = meta.metadata.context("Missing `meta` field")?;
        let name = meta.name;
        let target = meta.target;

        Ok(Self { id, name, target })
    }
}
