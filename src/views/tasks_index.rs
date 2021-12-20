use crate::{cancel_on_drop::CancelOnDrop, routes::ConsoleAddr};
use anyhow::Context as _;
use axum_liveview::{html, pubsub::Bincode, Html, LiveView, ShouldRender};
use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fmt,
};
use uuid::Uuid;

pub struct TasksIndex {
    _token: CancelOnDrop,
    addr: ConsoleAddr,
    id: Uuid,
    connection_state: ConnectionState,
    tasks: BTreeMap<Id, Task>,
    metadata: HashMap<MetaId, Metadata>,
}

impl LiveView for TasksIndex {
    fn setup(&self, setup: &mut axum_liveview::Setup<Self>) {
        setup.on_broadcast(&msg_topic(self.id), Self::msg);
        setup.on_broadcast("tick", Self::tick);
    }

    fn render(&self) -> Html {
        // TODO(david): extract this so we can also show it on the resources screen
        let connection_state = match &self.connection_state {
            ConnectionState::Connected => html! {
                "Connection: " { &self.addr.ip } ":" { &self.addr.port }
            },
            ConnectionState::StreamEnded => html! {
                "Stream ended unexpectedly..."
            },
            ConnectionState::Error { code, message } => html! {
                "Stream encountered an error:"
                <div>
                    <code>
                        "code=" { code } "; message=" { message }
                    </code>
                </div>
            },
        };

        let mut total = 0;
        let mut running = 0;
        let mut idle = 0;
        let mut dropped = 0;
        for task in self.tasks.values() {
            total += 1;
            match task.state() {
                TaskState::Running => running += 1,
                TaskState::Idle => idle += 1,
                TaskState::Dropped => dropped += 1,
            }
        }

        html! {
            <div>{ connection_state }</div>

            <div>
                if self.tasks.is_empty() {
                    "Loading..."
                } else {
                    <div>
                        "Tasks: " { total }

                        if running != 0 {
                            ", running: " { running }
                        }

                        if idle != 0 {
                            ", idle: " { idle }
                        }

                        if dropped != 0 {
                            ", dropped: " { dropped }
                        }
                    </div>
                    <hr />
                    <table class="tasks-table">
                        <thead>
                            <tr>
                                <th>"ID"</th>
                                <th>"State"</th>
                                <th>"Name"</th>
                                <th>"Total"</th>
                                <th>"Busy"</th>
                                <th>"Idle"</th>
                                <th>"Polls"</th>
                                <th>"Target"</th>
                                <th>"Location"</th>
                                <th>"Fields"</th>
                            </tr>
                        </thead>
                        <tbody>
                            for (_, task) in &self.tasks {
                                { task.render_as_table_row() }
                            }
                        </tbody>
                    </table>
                }
            </div>
        }
    }
}

impl TasksIndex {
    pub fn new(token: CancelOnDrop, addr: ConsoleAddr, id: Uuid) -> Self {
        Self {
            _token: token,
            addr,
            id,
            connection_state: ConnectionState::Connected,
            tasks: Default::default(),
            metadata: Default::default(),
        }
    }

    async fn msg(mut self, Bincode(msg): Bincode<Msg>) -> ShouldRender<Self> {
        match msg {
            Msg::Update(Update {
                new_tasks,
                stats_update,
                new_metadata,
            }) => {
                for task in new_tasks {
                    self.tasks.insert(task.id, task);
                }

                for (id, stats) in stats_update {
                    if let Some(task) = self.tasks.get_mut(&id) {
                        task.stats = Some(stats);
                    }
                }

                self.metadata.extend(new_metadata);

                for task in self.tasks.values_mut() {
                    if let Some(metadata) = self.metadata.get(&task.metadata_id) {
                        task.target = Some(metadata.target.clone());
                    }
                }
            }
            Msg::StreamEnded => {
                self.connection_state = ConnectionState::StreamEnded;
            }
            Msg::Error { code, message } => {
                self.connection_state = ConnectionState::Error { code, message };
            }
        }

        ShouldRender::No(self)
    }

    async fn tick(mut self) -> Self {
        self.reap_tasks();
        self
    }

    fn reap_tasks(&mut self) {
        self.tasks.retain(|_id, task| {
            if let Some(stats) = &task.stats {
                if let Some(dropped_at) = stats.dropped_at {
                    dropped_at > (Utc::now() - chrono::Duration::seconds(5))
                } else {
                    true
                }
            } else {
                true
            }
        })
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub enum Msg {
    Update(Update),
    StreamEnded,
    Error { code: i32, message: String },
}

pub fn msg_topic(id: Uuid) -> String {
    format!("tasks-index/msg/{}", id)
}

enum ConnectionState {
    Connected,
    StreamEnded,
    Error { code: i32, message: String },
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Update {
    new_tasks: Vec<Task>,
    stats_update: BTreeMap<Id, Stats>,
    new_metadata: HashMap<MetaId, Metadata>,
}

impl TryFrom<console_api::instrument::Update> for Update {
    type Error = anyhow::Error;

    fn try_from(update: console_api::instrument::Update) -> Result<Self, Self::Error> {
        let console_api::instrument::Update {
            now: _,
            task_update,
            resource_update: _,
            async_op_update: _,
            new_metadata,
        } = update;

        let new_metadata = new_metadata
            .unwrap_or_default()
            .metadata
            .into_iter()
            .map(|meta| Metadata::try_from(meta).map(|meta| (meta.id, meta)))
            .collect::<anyhow::Result<_>>()?;

        let console_api::tasks::TaskUpdate {
            new_tasks,
            stats_update,
            dropped_events: _,
        } = task_update.context("Missing `task_update` field")?;

        let new_tasks = new_tasks
            .into_iter()
            .map(TryInto::try_into)
            .collect::<anyhow::Result<_>>()?;

        let stats_update = stats_update
            .into_iter()
            .map(|(id, stats)| Ok((Id(id), stats.try_into()?)))
            .collect::<anyhow::Result<_>>()?;

        Ok(Self {
            new_tasks,
            stats_update,
            new_metadata,
        })
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

#[derive(Deserialize, Serialize, Debug)]
struct Task {
    id: Id,
    fields: BTreeMap<String, FieldValue>,
    location: Location,
    stats: Option<Stats>,
    metadata_id: MetaId,
    target: Option<String>,
}

impl TryFrom<console_api::tasks::Task> for Task {
    type Error = anyhow::Error;

    fn try_from(task: console_api::tasks::Task) -> Result<Self, Self::Error> {
        let console_api::tasks::Task {
            id,
            metadata,
            kind: _,
            fields,
            parents: _,
            location,
        } = task;

        let id = id.context("Missing `id` field")?;
        let id = Id(id.id);

        let metadata_id = MetaId(metadata.context("Missing `metadata` field")?.id);

        let fields = fields
            .into_iter()
            .map(|field| {
                let name = field.name.context("Missing `name` field")?;
                let value = field.value.context("Missing `value` field")?;

                let name = match name {
                    console_api::field::Name::StrName(name) => name,
                    console_api::field::Name::NameIdx(_) => {
                        tracing::warn!("hit NameIdx");
                        return Ok(None);
                    }
                };

                let value = FieldValue::from(value);

                Ok(Some((name, value)))
            })
            .filter_map(Result::transpose)
            .collect::<anyhow::Result<_>>()?;

        let location = location.context("Missing `location` field")?;
        let location = location.try_into()?;

        Ok(Self {
            id,
            fields,
            location,
            stats: None,
            metadata_id,
            target: None,
        })
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Id(u64);

impl Task {
    fn render_as_table_row(&self) -> Html {
        let state = match self.state() {
            TaskState::Running => "▶️",
            TaskState::Idle => "?",
            TaskState::Dropped => "⏹",
        };

        html! {
            <tr>
                <td>
                    { self.id.0 }
                </td>
                <td>{ state }</td>
                <td>
                    <code>
                        if let Some(name) = self.name() {
                            { name }
                        } else {
                            ""
                        }
                    </code>
                </td>
                // total
                <td>"..."</td>
                // busy
                <td>"..."</td>
                // idle
                <td>"..."</td>
                <td>
                    if let Some(stats) = &self.stats {
                        { stats.polls }
                    }
                </td>
                <td>
                    if let Some(target) = &self.target {
                        <code>{ target }</code>
                    }
                </td>
                <td>
                    <code>
                        { self.location.render() }
                    </code>
                </td>
                // fields
                <td>
                    for (name, value) in self.fields.iter().filter(|(name, _)| name != &"task.name") {
                        <code>
                            { format!("{}={}", name, value) }
                        </code>
                    }
                </td>
            </tr>
        }
    }

    fn name(&self) -> Option<&str> {
        match self.fields.get("task.name")? {
            FieldValue::Debug(name) => Some(name),
            FieldValue::Str(name) => Some(name),
            _ => None,
        }
    }

    fn state(&self) -> TaskState {
        if let Some(stats) = &self.stats {
            if stats.dropped_at.is_some() {
                TaskState::Dropped
            } else {
                TaskState::Running
            }
        } else {
            TaskState::Running
        }
    }
}

enum TaskState {
    Running,
    Idle,
    Dropped,
}

#[derive(Deserialize, Serialize, Debug)]
enum FieldValue {
    Debug(String),
    Str(String),
    U64(u64),
    I64(i64),
    Bool(bool),
}

impl fmt::Display for FieldValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldValue::Debug(inner) => inner.fmt(f),
            FieldValue::Str(inner) => inner.fmt(f),
            FieldValue::U64(inner) => inner.fmt(f),
            FieldValue::I64(inner) => inner.fmt(f),
            FieldValue::Bool(inner) => inner.fmt(f),
        }
    }
}

impl From<console_api::field::Value> for FieldValue {
    fn from(value: console_api::field::Value) -> Self {
        match value {
            console_api::field::Value::DebugVal(inner) => Self::Debug(inner),
            console_api::field::Value::StrVal(inner) => Self::Str(inner),
            console_api::field::Value::U64Val(inner) => Self::U64(inner),
            console_api::field::Value::I64Val(inner) => Self::I64(inner),
            console_api::field::Value::BoolVal(inner) => Self::Bool(inner),
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct Location {
    file: String,
    module_path: Option<String>,
    line: u32,
    column: u32,
}

impl Location {
    fn render(&self) -> Html {
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

#[derive(Deserialize, Serialize, Debug)]
struct Stats {
    #[serde(with = "chrono::serde::ts_nanoseconds")]
    created_at: DateTime<Utc>,
    #[serde(with = "chrono::serde::ts_nanoseconds_option")]
    dropped_at: Option<DateTime<Utc>>,
    wakes: u64,
    waker_clones: u64,
    waker_drops: u64,
    #[serde(with = "chrono::serde::ts_nanoseconds_option")]
    last_wake: Option<DateTime<Utc>>,
    self_wakes: u64,
    polls: u64,
    // first_poll: Option<Timestamp>,
    // last_poll_started: Option<Timestamp>,
    // last_poll_ended: Option<Timestamp>,
    // busy_time: Option<Duration>,
}

impl TryFrom<console_api::tasks::Stats> for Stats {
    type Error = anyhow::Error;

    fn try_from(stats: console_api::tasks::Stats) -> Result<Self, Self::Error> {
        let console_api::tasks::Stats {
            created_at,
            dropped_at,
            wakes,
            waker_clones,
            waker_drops,
            last_wake,
            self_wakes,
            poll_stats: _,
        } = stats;

        let created_at = created_at.context("Missing `created_at` field")?;
        let created_at = Utc.timestamp(created_at.seconds, created_at.nanos as _);

        let dropped_at =
            dropped_at.map(|dropped_at| Utc.timestamp(dropped_at.seconds, dropped_at.nanos as _));

        let last_wake =
            last_wake.map(|last_wake| Utc.timestamp(last_wake.seconds, last_wake.nanos as _));

        let poll_stats = stats.poll_stats.context("Missing `poll_stats` field")?;

        Ok(Self {
            created_at,
            dropped_at,
            wakes,
            waker_clones,
            waker_drops,
            last_wake,
            self_wakes,
            polls: poll_stats.polls,
        })
    }
}
