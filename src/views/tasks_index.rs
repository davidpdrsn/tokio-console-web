use super::{
    table::TableView,
    table_view_keybinds::{TableViewKeybinds, TableViewKeybindsUpdate},
    StateRef,
};
use crate::{
    routes::ConsoleAddr,
    watch_stream::{ConsoleState, ConsoleStateWatch, Task, TaskId, TaskState},
};
use axum::{
    async_trait,
    http::{HeaderMap, Uri},
};
use axum_live_view::{
    event_data::EventData,
    html,
    js_command::{self, JsCommand},
    live_view::{Updated, ViewHandle},
    Html, LiveView,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Duration};

pub struct TasksIndex {
    rx: ConsoleStateWatch,
    paused_state: Option<ConsoleState>,
    addr: ConsoleAddr,
    connected: bool,
    runtime_stats: HashMap<TaskId, TaskRuntimeStats>,
    tally: Tally,
    table_keybinds: TableViewKeybinds,
}

impl TasksIndex {
    pub fn new(addr: ConsoleAddr, rx: ConsoleStateWatch) -> Self {
        Self {
            addr,
            rx,
            paused_state: None,
            connected: true,
            runtime_stats: Default::default(),
            tally: Default::default(),
            table_keybinds: Default::default(),
        }
    }
}

impl TasksIndex {
    fn state(&self) -> StateRef<'_, ConsoleState> {
        let state = self.rx.borrow();
        if let Some(state) = &self.paused_state {
            StateRef::Ref(state)
        } else {
            StateRef::BorrowedFromWatch(state)
        }
    }
}

#[derive(Default)]
struct Tally {
    total: usize,
    running: usize,
    idle: usize,
    completed: usize,
}

#[async_trait]
impl LiveView for TasksIndex {
    type Message = Msg;
    type Error = anyhow::Error;

    async fn mount(
        &mut self,
        _uri: Uri,
        _request_headers: &HeaderMap,
        handle: ViewHandle<Self::Message>,
    ) -> Result<(), Self::Error> {
        let mut rx = self.rx.clone();
        tokio::spawn(async move {
            loop {
                if rx.changed().await.is_err() {
                    break;
                }
                if handle.send(Msg::Update).await.is_err() {
                    break;
                }
            }
            let _ = handle.send(Msg::Disconnected).await;
            let _ = handle.send(Msg::Error).await;
        });
        Ok(())
    }

    async fn update(
        mut self,
        msg: Self::Message,
        data: Option<EventData>,
    ) -> Result<Updated<Self>, Self::Error> {
        self.do_update(msg, data).await
    }

    fn render(&self) -> Html<Self::Message> {
        html! {
            if self.connected {
                <div>
                    "Connection: " { &self.addr.ip } ":" { &self.addr.port }
                </div>
            } else {
                <div>
                    "Not connected..."
                </div>
            }

            { self.table_keybinds.help() }

            <div>
                "Tasks: " { self.tally.total }

                if self.tally.running != 0 {
                    ", running: " { self.tally.running }
                }

                if self.tally.idle != 0 {
                    ", idle: " { self.tally.idle }
                }

                if self.tally.completed != 0 {
                    ", completed: " { self.tally.completed }
                }
            </div>

            <div>
                if self.paused_state.is_some() {
                    <button axm-click={ Msg::TogglePlayPause }>"Play"</button>
                } else {
                    <button axm-click={ Msg::TogglePlayPause }>"Pause"</button>
                }
            </div>

            { self.table_render() }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum Msg {
    TogglePlayPause,
    RowClick(TaskId),
    Update,
    Disconnected,
    Error,
    Key,
}

impl TasksIndex {
    async fn do_update(
        mut self,
        msg: Msg,
        data: Option<EventData>,
    ) -> Result<Updated<Self>, anyhow::Error> {
        let mut commands = Vec::new();

        match msg {
            Msg::TogglePlayPause => {
                self.toggle_play_pause();
            }
            Msg::RowClick(task_id) => {
                commands.push(self.navigate_to_task_command(task_id));
            }
            Msg::Key => match self.table_keybinds.update(data.as_ref()) {
                Some(TableViewKeybindsUpdate::Selected(idx)) => {
                    if let Some(id) = self.selected_task(idx) {
                        commands.push(self.navigate_to_task_command(id));
                    }
                }
                Some(TableViewKeybindsUpdate::GotoTasks) => {}
                Some(TableViewKeybindsUpdate::GotoResources) => {
                    commands.push(js_command::navigate_to(
                        format!("/console/{}/{}/resources", self.addr.ip, self.addr.port)
                            .parse()
                            .unwrap(),
                    ));
                }
                Some(TableViewKeybindsUpdate::TogglePlayPause) => {
                    self.toggle_play_pause();
                }
                None => {}
            },
            Msg::Update => {
                if self.paused_state.is_none() {
                    self.tally = Default::default();

                    for task in self.rx.borrow().tasks.values() {
                        let mut times = TaskRuntimeStats::default();

                        if let Some(total) = task
                            .stats
                            .as_ref()
                            .and_then(|s| s.created_at)
                            .map(|t| t.elapsed().unwrap())
                        {
                            times.total = Some(total);
                        }

                        if let Some(busy) = task.stats.as_ref().and_then(|s| s.busy_time) {
                            times.busy = Some(busy);
                        }

                        if let Some(idle) = task.stats.as_ref().and_then(|s| s.idle_time()) {
                            times.idle = Some(idle);
                        }

                        self.runtime_stats.insert(task.id, times);

                        self.tally.total += 1;
                        match task.state() {
                            TaskState::Running => self.tally.running += 1,
                            TaskState::Idle => self.tally.idle += 1,
                            TaskState::Completed => self.tally.completed += 1,
                        }
                    }
                }
            }
            Msg::Disconnected => {
                self.connected = false;
            }
            Msg::Error => {
                anyhow::bail!("console subscription disconnected")
            }
        };

        let num_tasks = self.state().tasks.len();
        self.table_keybinds.clamp_selected_idx(num_tasks);

        Ok(Updated::new(self).with_all(commands))
    }

    fn selected_task(&self, idx: usize) -> Option<TaskId> {
        let state = self.state();
        let task = state.tasks.values().nth(idx)?;
        Some(task.id)
    }

    fn navigate_to_task_command(&self, id: TaskId) -> JsCommand {
        let uri = format!(
            "/console/{}/{}/tasks/{}",
            self.addr.ip, self.addr.port, id.0
        )
        .parse()
        .expect("invalid URI");
        js_command::navigate_to(uri)
    }

    fn toggle_play_pause(&mut self) {
        if self.paused_state.is_some() {
            self.paused_state = None;
        } else {
            self.paused_state = Some(self.rx.borrow().clone());
        }
    }
}

pub(crate) struct TaskViewModel {
    task: Arc<Task>,
    runtime_stats: Option<TaskRuntimeStats>,
}

#[derive(Default, Clone, Copy)]
struct TaskRuntimeStats {
    total: Option<Duration>,
    busy: Option<Duration>,
    idle: Option<Duration>,
}

impl TableView for TasksIndex {
    type Column = Column;
    type Model = TaskViewModel;
    type Msg = Msg;

    fn columns(&self) -> Vec<Self::Column> {
        Column::all()
    }

    fn rows(&self) -> Vec<Self::Model> {
        self.state()
            .tasks
            .values()
            .map(|task| TaskViewModel {
                task: Arc::clone(task),
                runtime_stats: self.runtime_stats.get(&task.id).copied(),
            })
            .collect()
    }

    fn row_click_event(&self, row: &TaskViewModel) -> Self::Msg {
        Msg::RowClick(row.task.id)
    }

    fn key_event(&self) -> Self::Msg {
        Msg::Key
    }

    fn row_selected(&self, idx: usize, _: &Self::Model) -> bool {
        self.table_keybinds.selected_idx() == Some(idx)
    }

    fn render_column(&self, col: &Self::Column, row: &TaskViewModel) -> Html<Self::Msg> {
        match col {
            Column::ID => {
                html! { { row.task.id.0 } }
            }
            Column::State => {
                let state = match row.task.state() {
                    TaskState::Running => "▶️",
                    TaskState::Idle => "⏸",
                    TaskState::Completed => "⏹",
                };

                html! { { state } }
            }
            Column::Name => {
                html! {
                    <code>
                        if let Some(name) = row.task.name() {
                            { name }
                        } else {
                            ""
                        }
                    </code>
                }
            }
            Column::Total => {
                html! {
                    if let Some(total) = row.runtime_stats.and_then(|t| t.total) {
                        { format!("{:?}", total) }
                    }
                }
            }
            Column::Busy => {
                html! {
                    if let Some(busy) = row.runtime_stats.and_then(|t| t.busy) {
                        { format!("{:?}", busy) }
                    }
                }
            }
            Column::Idle => {
                html! {
                    if let Some(idle) = row.runtime_stats.and_then(|t| t.idle) {
                        { format!("{:?}", idle) }
                    }
                }
            }
            Column::Polls => {
                html! {
                    if let Some(stats) = &row.task.stats {
                        { stats.polls }
                    }
                }
            }
            Column::Target => {
                html! {
                    if let Some(target) = &row.task.target {
                        <code>{ target }</code>
                    }
                }
            }
            Column::Location => {
                html! {
                    <code>
                        { row.task.location.render() }
                    </code>
                }
            }
            Column::Fields => {
                html! {
                    for (name, value) in row.task.fields.iter().filter(|(name, _)| name != &"task.name") {
                        <code>
                            { format!("{}={}", name, value) }
                        </code>
                    }
                }
            }
        }
    }
}

columns_enum! {
    pub(crate) enum Column {
        ID,
        State,
        Name,
        Total,
        Busy,
        Idle,
        Polls,
        Target,
        Location,
        Fields,
    }
}
