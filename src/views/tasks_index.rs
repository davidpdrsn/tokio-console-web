use std::{
    collections::HashMap,
    time::{Duration, Instant},
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

pub struct TasksIndex {
    rx: ConsoleStateWatch,
    paused_state: Option<ConsoleState>,
    addr: ConsoleAddr,
    connected: bool,
    selected_idx: Option<usize>,
    show_key_binds: bool,
    prev_key_event: Instant,
    task_runtime_stats: HashMap<TaskId, TaskRuntimeStats>,
}

impl TasksIndex {
    pub fn new(addr: ConsoleAddr, rx: ConsoleStateWatch) -> Self {
        Self {
            addr,
            rx,
            paused_state: None,
            connected: true,
            selected_idx: None,
            show_key_binds: false,
            prev_key_event: Instant::now(),
            task_runtime_stats: Default::default(),
        }
    }
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
        let state = self.rx.borrow();
        let state = if let Some(state) = &self.paused_state {
            state
        } else {
            &*state
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
            if self.connected {
                <div>
                    "Connection: " { &self.addr.ip } ":" { &self.addr.port }
                </div>
            } else {
                <div>
                    "Not connected..."
                </div>
            }

            if self.show_key_binds {
                <div class="keybinds">
                    "Key binds<br>"
                    "j/k: down/up<br>"
                    "space: play/pause<br>"
                    "enter: open task<br>"
                    "?: show/hide keybinds"
                </div>
            }

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

            <table
                class="tasks-table"
                axm-window-keydown={ Msg::Key }
            >
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
                    for (idx, task) in state.tasks.values().enumerate() {
                        {
                            task.render_as_table_row(
                                Some(idx) == self.selected_idx,
                                self.task_runtime_stats.get(&task.id),
                            )
                        }
                    }
                </tbody>
            </table>
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
                commands.push(self.navigate_to_ask_command(task_id));
            }
            Msg::Key => {
                if self.prev_key_event.elapsed() > Duration::from_millis(50) {
                    self.prev_key_event = Instant::now();

                    let data = data.unwrap();
                    let key = data.as_key().unwrap().key();

                    match key {
                        "k" => {
                            if let Some(idx) = self.selected_idx.as_mut() {
                                if *idx != 0 {
                                    *idx -= 1;
                                }
                            }
                        }
                        "j" => {
                            if let Some(idx) = self.selected_idx.as_mut() {
                                *idx += 1;
                            } else {
                                self.selected_idx = Some(0);
                            }
                        }
                        "Enter" => {
                            if let Some(id) = self.selected_task() {
                                commands.push(self.navigate_to_ask_command(id));
                            }
                        }
                        " " => {
                            self.toggle_play_pause();
                        }
                        "?" => {
                            self.show_key_binds = !self.show_key_binds;
                        }
                        _ => {}
                    }
                }
            }
            Msg::Update => {
                if self.paused_state.is_none() {
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

                        self.task_runtime_stats.insert(task.id, times);
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

        if let Some(idx) = self.selected_idx.as_mut() {
            let num_tasks = if let Some(state) = &self.paused_state {
                state.tasks.len()
            } else {
                self.rx.borrow().tasks.len()
            };
            *idx = std::cmp::min(num_tasks - 1, *idx);
        }

        Ok(Updated::new(self).with_all(commands))
    }

    fn selected_task(&self) -> Option<TaskId> {
        let idx = self.selected_idx?;
        let state = self.rx.borrow();
        let state = if let Some(state) = &self.paused_state {
            state
        } else {
            &*state
        };
        let task = state.tasks.values().nth(idx)?;
        Some(task.id)
    }

    fn navigate_to_ask_command(&self, id: TaskId) -> JsCommand {
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

impl Task {
    fn render_as_table_row(
        &self,
        selected: bool,
        runtime_stats: Option<&TaskRuntimeStats>,
    ) -> Html<Msg> {
        let state = match self.state() {
            TaskState::Running => "▶️",
            TaskState::Idle => "⏸",
            TaskState::Completed => "⏹",
        };

        html! {
            <tr
                axm-click={ Msg::RowClick(self.id) }
                class=if selected { "row-selected" }
            >
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
                    if let Some(total) = runtime_stats.and_then(|t| t.total) {
                        { format!("{:?}", total) }
                    }
                </td>
                <td>
                    if let Some(busy) = runtime_stats.and_then(|t| t.busy) {
                        { format!("{:?}", busy) }
                    }
                </td>
                <td>
                    if let Some(idle) = runtime_stats.and_then(|t| t.idle) {
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
}

#[derive(Default)]
struct TaskRuntimeStats {
    total: Option<Duration>,
    busy: Option<Duration>,
    idle: Option<Duration>,
}
