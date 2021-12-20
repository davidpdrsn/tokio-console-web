use crate::routes::ConsoleAddr;
use axum_liveview::{html, pubsub::Bincode, LiveView};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub struct ConnectionState {
    stream_id: Uuid,
    state: State,
    addr: ConsoleAddr,
}

impl ConnectionState {
    pub fn new(stream_id: Uuid, addr: ConsoleAddr) -> Self {
        Self {
            stream_id,
            state: State::Connected,
            addr,
        }
    }
}

impl LiveView for ConnectionState {
    fn setup(&self, setup: &mut axum_liveview::Setup<Self>) {
        setup.on_broadcast(&msg_topic(self.stream_id), Self::msg);
    }

    fn render(&self) -> axum_liveview::Html {
        let connection_state = match &self.state {
            State::Connected => html! {
                "Connection: " { &self.addr.ip } ":" { &self.addr.port }
            },
            State::StreamEnded => html! {
                "Stream ended unexpectedly..."
            },
            State::Error { code, message } => html! {
                "Stream encountered an error:"
                <div>
                    <code>
                        "code=" { code } "; message=" { message }
                    </code>
                </div>
            },
        };

        html! {
            <div>
                { connection_state }
            </div>
        }
    }
}

enum State {
    Connected,
    StreamEnded,
    Error { code: i32, message: String },
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Msg {
    StreamEnded,
    Error { code: i32, message: String },
}

impl ConnectionState {
    async fn msg(mut self, Bincode(msg): Bincode<Msg>) -> Self {
        match msg {
            Msg::StreamEnded => self.state = State::StreamEnded,
            Msg::Error { code, message } => self.state = State::Error { code, message },
        }

        self
    }
}

pub fn msg_topic(id: Uuid) -> String {
    format!("connection-state/msg/{}", id)
}
