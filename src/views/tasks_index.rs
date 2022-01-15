use super::{Location, MetaId, Metadata};
use crate::{cancel_on_drop::CancelOnDrop, routes::ConsoleAddr};
use anyhow::Context as _;
use axum::{
    async_trait,
    http::{HeaderMap, Uri},
};
use axum_live_view::{
    event_data::EventData,
    html, js_command,
    live_view::{Updated, ViewHandle},
    Html, LiveView,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    convert::Infallible,
    fmt,
    time::{Duration, SystemTime},
};
use tokio::sync::mpsc;

pub struct TasksIndex {
    state: State,
    paused_state: Option<State>,
    addr: ConsoleAddr,
    rx: Option<mpsc::Receiver<Update>>,
    _token: CancelOnDrop,
}

#[derive(Clone)]
struct State {
    tasks: BTreeMap<TaskId, Task>,
    metadata: HashMap<MetaId, Metadata>,
}

#[async_trait]
impl LiveView for TasksIndex {
    type Message = Msg;
    type Error = Infallible;

    async fn mount(
        &mut self,
        _uri: Uri,
        _request_headers: &HeaderMap,
        handle: ViewHandle<Self::Message>,
    ) -> Result<(), Self::Error> {
        if let Some(mut rx) = self.rx.take() {
            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    if handle.send(Msg::Update(msg)).await.is_err() {
                        break;
                    }
                }
            });
        }
        Ok(())
    }

    async fn update(
        mut self,
        msg: Self::Message,
        _data: Option<EventData>,
    ) -> Result<Updated<Self>, Self::Error> {
        match msg {
            Msg::TogglePlayPause => {
                if self.paused_state.is_some() {
                    self.paused_state = None;
                } else {
                    self.paused_state = Some(self.state.clone());
                }
                Ok(Updated::new(self))
            }
            Msg::RowClick(task_id) => {
                let uri = format!(
                    "/console/{}/{}/tasks/{}",
                    self.addr.ip, self.addr.port, task_id.0
                )
                .parse()
                .expect("invalid URI");

                Ok(Updated::new(self).with(js_command::navigate_to(uri)))
            }
            Msg::Update(update) => {
                let Update {
                    new_tasks,
                    stats_update,
                    new_metadata,
                } = update;

                for task in new_tasks {
                    self.state.tasks.insert(task.id, task);
                }

                for (id, stats) in stats_update {
                    if let Some(task) = self.state.tasks.get_mut(&id) {
                        task.stats = Some(stats);
                    }
                }

                self.state.metadata.extend(new_metadata);

                for task in self.state.tasks.values_mut() {
                    if let Some(metadata) = self.state.metadata.get(&task.metadata_id) {
                        task.target = Some(metadata.target.clone());
                    }
                }

                self.state.tasks.retain(|_id, task| {
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

                Ok(Updated::new(self))
            }
        }
    }

    fn render(&self) -> Html<Self::Message> {
        let state = if let Some(state) = &self.paused_state {
            state
        } else {
            &self.state
        };

        let mut total = 0;
        let mut running = 0;
        let mut idle = 0;
        let mut completed = 0;

        for task in state.tasks.values() {
            total += 1;
            match task.state() {
                TaskState::Running => running += 1,
                TaskState::Idle => idle += 1,
                TaskState::Completed => completed += 1,
            }
        }

        html! {
            <div>
                if state.tasks.is_empty() {
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

                        if completed != 0 {
                            ", completed: " { completed }
                        }
                    </div>
                    <div>
                        if self.paused_state.is_some() {
                            <button axm-click={ Msg::TogglePlayPause }>"Play"</button>
                        } else {
                            <button axm-click={ Msg::TogglePlayPause }>"Pause"</button>
                        }
                    </div>
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
                            for task in state.tasks.values() {
                                { task.render_as_table_row() }
                            }
                        </tbody>
                    </table>
                }
            </div>
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum Msg {
    TogglePlayPause,
    RowClick(TaskId),
    Update(Update),
}

impl TasksIndex {
    pub fn new(addr: ConsoleAddr, token: CancelOnDrop, rx: mpsc::Receiver<Update>) -> Self {
        Self {
            state: State {
                tasks: Default::default(),
                metadata: Default::default(),
            },
            paused_state: None,
            addr,
            _token: token,
            rx: Some(rx),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
pub struct Update {
    new_tasks: Vec<Task>,
    stats_update: BTreeMap<TaskId, Stats>,
    new_metadata: HashMap<MetaId, Metadata>,
}

impl TryFrom<console_api::instrument::Update> for Update {
    type Error = anyhow::Error;

    fn try_from(update: console_api::instrument::Update) -> Result<Self, Self::Error> {
        let console_api::instrument::Update {
            task_update,
            new_metadata,
            ..
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
            .map(|(id, stats)| Ok((TaskId(id), stats.try_into()?)))
            .collect::<anyhow::Result<_>>()?;

        Ok(Self {
            new_tasks,
            stats_update,
            new_metadata,
        })
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
struct Task {
    id: TaskId,
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
pub struct TaskId(u64);

impl Task {
    fn render_as_table_row(&self) -> Html<Msg> {
        let state = match self.state() {
            TaskState::Running => "▶️",
            TaskState::Idle => "⏸",
            TaskState::Completed => "⏹",
        };

        html! {
            <tr axm-click={ Msg::RowClick(self.id) }>
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
                <td>
                    if let Some(created_at) = self.stats.as_ref().and_then(|s| s.created_at) {
                        { format!("{:?}", created_at.elapsed().unwrap()) }
                    }
                </td>
                <td>
                    if let Some(busy) = self.stats.as_ref().and_then(|s| s.busy_time) {
                        { format!("{:?}", busy) }
                    }
                </td>
                <td>
                    if let Some(idle) = self.stats.as_ref().and_then(|s| s.idle_time()) {
                        { format!("{:?}", idle) }
                    }
                </td>
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
        if self.is_completed() {
            return TaskState::Completed;
        }

        if self.is_running() {
            return TaskState::Running;
        }

        TaskState::Idle
    }

    fn is_completed(&self) -> bool {
        let stats = if let Some(stats) = &self.stats {
            stats
        } else {
            return false;
        };

        stats.dropped_at.is_some()
    }

    fn is_running(&self) -> bool {
        let stats = if let Some(stats) = &self.stats {
            stats
        } else {
            return false;
        };

        stats.last_poll_started > stats.last_poll_ended
    }
}

enum TaskState {
    Running,
    Idle,
    Completed,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
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

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
struct Stats {
    dropped_at: Option<SystemTime>,
    created_at: Option<SystemTime>,
    busy_time: Option<Duration>,
    last_poll_started: Option<Duration>,
    last_poll_ended: Option<Duration>,
    polls: u64,
}

impl TryFrom<console_api::tasks::Stats> for Stats {
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

impl Stats {
    fn idle_time(&self) -> Option<Duration> {
        let created_at = self.created_at?.elapsed().ok()?;
        let busy_time = self.busy_time?;
        Some(created_at - busy_time)
    }
}
