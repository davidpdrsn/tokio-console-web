#![allow(dead_code)]

use crate::{routes::ConsoleAddr, InstrumentClient};
use anyhow::Context as _;
use console_api::instrument::InstrumentRequest;
use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::Entry, BTreeMap, HashMap},
    fmt,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::sync::{watch, Mutex};
use tonic::{transport::Endpoint, Streaming};

#[derive(Clone, Default)]
pub struct ConsoleSubscriptions {
    inner: Arc<Mutex<HashMap<ConsoleAddr, ConsoleStateWatch>>>,
}

impl ConsoleSubscriptions {
    pub async fn subscribe(&self, addr: ConsoleAddr) -> anyhow::Result<ConsoleStateWatch> {
        let map = self.inner.clone();

        match self.inner.lock().await.entry(addr.clone()) {
            Entry::Occupied(entry) => {
                tracing::debug!(?addr, "reusing existing subscription");
                Ok(entry.get().clone())
            }
            Entry::Vacant(entry) => {
                let endpoint = format!("http://{}:{}", addr.ip, addr.port).parse::<Endpoint>()?;

                let channel = endpoint.connect().await?;
                let mut client = InstrumentClient::new(channel);

                let stream = client
                    .watch_updates(InstrumentRequest {})
                    .await?
                    .into_inner();

                let (tx, rx) = watch::channel(ConsoleState::default());

                tokio::spawn(async move {
                    tracing::debug!(?addr, "creating subscription for");
                    match subscribe_to_console_updates(stream, tx).await {
                        Ok(()) => {
                            tracing::debug!(?addr, "watch stream ended");
                        }
                        Err(err) => {
                            tracing::error!(%err, "console watch stream ended");
                        }
                    }
                    map.lock().await.remove(&addr);
                });

                let watch = ConsoleStateWatch { rx };
                entry.insert(watch.clone());
                Ok(watch)
            }
        }
    }
}

async fn subscribe_to_console_updates(
    mut stream: Streaming<console_api::instrument::Update>,
    tx: watch::Sender<ConsoleState>,
) -> anyhow::Result<()> {
    let mut state = ConsoleState::default();

    while let Ok(Some(msg)) = stream.message().await {
        #[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
        pub struct Update {
            new_tasks: Vec<Task>,
            stats_update: BTreeMap<TaskId, TaskStats>,
            new_metadata: HashMap<MetaId, Metadata>,
        }

        let console_api::instrument::Update {
            task_update,
            new_metadata,
            resource_update,
            ..
        } = msg;

        // update metadata
        for new_metadata in new_metadata.unwrap_or_default().metadata {
            let metadata = Metadata::try_from(new_metadata)?;
            state.metadata.insert(metadata.id, metadata);
        }

        // update tasks
        {
            let console_api::tasks::TaskUpdate {
                new_tasks,
                stats_update,
                dropped_events: _,
            } = task_update.context("Missing `task_update` field")?;

            for new_task in new_tasks {
                let task = Task::try_from(new_task)?;
                state.tasks.insert(task.id, task);
            }

            for (id, stats) in stats_update {
                let id = TaskId(id);
                let stats = TaskStats::try_from(stats)?;
                if let Some(task) = state.tasks.get_mut(&id) {
                    task.stats = Some(stats);
                }
            }

            for task in state.tasks.values_mut() {
                if let Some(metadata) = state.metadata.get(&task.metadata_id) {
                    task.target = Some(metadata.target.clone());
                }
            }
        }

        // update resources
        {
            let console_api::resources::ResourceUpdate {
                new_resources,
                stats_update,
                new_poll_ops: _,
                dropped_events: _,
            } = resource_update.context("Missing `resource_update` field")?;

            for new_resource in new_resources {
                let resource = Resource::try_from(new_resource)?;
                state.resources.insert(resource.id, resource);
            }

            for (id, stats) in stats_update {
                let id = ResourceId(id);
                let stats = ResourceStats::try_from(stats)?;
                if let Some(task) = state.resources.get_mut(&id) {
                    task.stats = Some(stats);
                }
            }

            for resource in state.resources.values_mut() {
                if let Some(metadata) = state.metadata.get(&resource.metadata_id) {
                    resource.target = Some(metadata.target.clone());
                }
            }
        }

        // reap tasks
        state.tasks.retain(|_id, task| {
            if let Some(stats) = &task.stats {
                if let Some(dropped_at) = stats.dropped_at {
                    dropped_at.elapsed().unwrap() < Duration::from_secs(5)
                } else {
                    true
                }
            } else {
                true
            }
        });

        // reap resources
        state.resources.retain(|_id, resource| {
            if let Some(stats) = &resource.stats {
                if let Some(dropped_at) = stats.dropped_at {
                    dropped_at.elapsed().unwrap() < Duration::from_secs(5)
                } else {
                    true
                }
            } else {
                true
            }
        });

        // notify subscribers
        tx.send(state.clone())
            .map_err(|_| anyhow::Error::msg("failed to send new state"))?;
    }

    Ok(())
}

#[derive(Clone)]
pub struct ConsoleStateWatch {
    rx: watch::Receiver<ConsoleState>,
}

impl ConsoleStateWatch {
    pub fn borrow(&self) -> watch::Ref<'_, ConsoleState> {
        self.rx.borrow()
    }

    pub async fn changed(&mut self) -> anyhow::Result<()> {
        Ok(self.rx.changed().await?)
    }
}

#[derive(Default, Clone, Debug)]
pub struct ConsoleState {
    pub tasks: BTreeMap<TaskId, Task>,
    pub resources: BTreeMap<ResourceId, Resource>,
    pub metadata: HashMap<MetaId, Metadata>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(pub u64);

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct Task {
    pub id: TaskId,
    pub fields: BTreeMap<String, FieldValue>,
    pub location: Location,
    pub stats: Option<TaskStats>,
    pub metadata_id: MetaId,
    pub target: Option<String>,
}

impl Task {
    pub fn name(&self) -> Option<&str> {
        match self.fields.get("task.name")? {
            FieldValue::Debug(name) => Some(name),
            FieldValue::Str(name) => Some(name),
            _ => None,
        }
    }

    pub fn state(&self) -> TaskState {
        if self.is_completed() {
            return TaskState::Completed;
        }

        if self.is_running() {
            return TaskState::Running;
        }

        TaskState::Idle
    }

    pub fn is_completed(&self) -> bool {
        let stats = if let Some(stats) = &self.stats {
            stats
        } else {
            return false;
        };

        stats.dropped_at.is_some()
    }

    pub fn is_running(&self) -> bool {
        let stats = if let Some(stats) = &self.stats {
            stats
        } else {
            return false;
        };

        stats.last_poll_started > stats.last_poll_ended
    }
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
        let id = TaskId(id.id);

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
pub struct MetaId(u64);

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
pub struct Metadata {
    pub id: MetaId,
    pub name: String,
    pub target: String,
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

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum FieldValue {
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

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
pub struct TaskStats {
    pub dropped_at: Option<SystemTime>,
    pub created_at: Option<SystemTime>,
    pub busy_time: Option<Duration>,
    pub last_poll_started: Option<Duration>,
    pub last_poll_ended: Option<Duration>,
    pub polls: u64,
}

impl TryFrom<console_api::tasks::Stats> for TaskStats {
    type Error = anyhow::Error;

    fn try_from(stats: console_api::tasks::Stats) -> Result<Self, Self::Error> {
        let console_api::tasks::Stats {
            created_at,
            dropped_at,
            wakes: _,
            waker_clones: _,
            waker_drops: _,
            last_wake: _,
            self_wakes: _,
            poll_stats,
        } = stats;

        let created_at = created_at.map(SystemTime::try_from).transpose()?;
        let dropped_at = dropped_at.map(SystemTime::try_from).transpose()?;

        let poll_stats = poll_stats.context("Missing `poll_stats` field")?;

        let polls = poll_stats.polls;
        let busy_time = poll_stats
            .busy_time
            .map(|d| Duration::new(d.seconds as _, d.nanos as _));

        let last_poll_started = poll_stats
            .last_poll_started
            .map(|d| Duration::new(d.seconds as _, d.nanos as _));

        let last_poll_ended = poll_stats
            .last_poll_ended
            .map(|d| Duration::new(d.seconds as _, d.nanos as _));

        Ok(Self {
            dropped_at,
            created_at,
            polls,
            busy_time,
            last_poll_started,
            last_poll_ended,
        })
    }
}

impl TaskStats {
    pub fn idle_time(&self) -> Option<Duration> {
        let created_at = self.created_at?.elapsed().ok()?;
        let busy_time = self.busy_time?;
        Some(created_at - busy_time)
    }
}

pub enum TaskState {
    Running,
    Idle,
    Completed,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct Location {
    pub file: String,
    pub module_path: Option<String>,
    pub line: u32,
    pub column: u32,
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
pub struct ResourceId(pub u64);

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
pub struct Resource {
    pub id: ResourceId,
    pub vis: TypeVisibility,
    pub parent_id: Option<ResourceId>,
    pub kind: String,
    pub concrete_type: String,
    pub location: Option<Location>,
    pub metadata_id: MetaId,
    pub target: Option<String>,
    pub stats: Option<ResourceStats>,
}

impl TryFrom<console_api::resources::Resource> for Resource {
    type Error = anyhow::Error;

    fn try_from(resource: console_api::resources::Resource) -> Result<Self, Self::Error> {
        let id = ResourceId(resource.id.context("Missing `id` field")?.id);

        let metadata_id = MetaId(resource.metadata.context("Missing `metadata` field")?.id);

        let kind = match resource
            .kind
            .context("Missing `kind` field")?
            .kind
            .context("Missing `kind.kind`")?
        {
            console_api::resources::resource::kind::Kind::Known(n) => {
                match console_api::resources::resource::kind::Known::from_i32(n).unwrap() {
                    console_api::resources::resource::kind::Known::Timer => "Timer".to_string(),
                }
            }
            console_api::resources::resource::kind::Kind::Other(s) => s,
        };

        let vis = if resource.is_internal {
            TypeVisibility::Internal
        } else {
            TypeVisibility::Public
        };

        let parent_id = resource.parent_resource_id.map(|id| ResourceId(id.id));

        let concrete_type = resource.concrete_type;

        let location = resource.location.map(Location::try_from).transpose()?;

        Ok(Self {
            id,
            metadata_id,
            vis,
            parent_id,
            kind,
            concrete_type,
            location,
            target: None,
            stats: None,
        })
    }
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone, Copy)]
pub enum TypeVisibility {
    Public,
    Internal,
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
pub struct ResourceStats {
    pub dropped_at: Option<SystemTime>,
    pub created_at: Option<SystemTime>,
}

impl TryFrom<console_api::resources::Stats> for ResourceStats {
    type Error = anyhow::Error;

    fn try_from(stats: console_api::resources::Stats) -> Result<Self, Self::Error> {
        let console_api::resources::Stats {
            dropped_at,
            created_at,
            attributes: _,
        } = stats;

        let created_at = created_at.map(SystemTime::try_from).transpose()?;
        let dropped_at = dropped_at.map(SystemTime::try_from).transpose()?;

        Ok(Self {
            dropped_at,
            created_at,
        })
    }
}
