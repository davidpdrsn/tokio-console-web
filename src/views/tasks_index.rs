use crate::{
    routes::ConsoleAddr,
    state::{ConsoleState, ConsoleStateWatch, Task, TaskId, TaskState},
};
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

pub struct TasksIndex {
    rx: ConsoleStateWatch,
    paused_state: Option<ConsoleState>,
    addr: ConsoleAddr,
    connected: bool,
}

impl TasksIndex {
    pub fn new(addr: ConsoleAddr, rx: ConsoleStateWatch) -> Self {
        Self {
            addr,
            rx,
            paused_state: None,
            connected: true,
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
        _data: Option<EventData>,
    ) -> Result<Updated<Self>, Self::Error> {
        match msg {
            Msg::TogglePlayPause => {
                if self.paused_state.is_some() {
                    self.paused_state = None;
                } else {
                    self.paused_state = Some(self.rx.borrow().clone());
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
            Msg::Update => Ok(Updated::new(self)),
            Msg::Disconnected => {
                self.connected = false;
                Ok(Updated::new(self))
            }
            Msg::Error => {
                anyhow::bail!("console subscription disconnected")
            }
        }
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
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum Msg {
    TogglePlayPause,
    RowClick(TaskId),
    Update,
    Disconnected,
    Error,
}

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
}
