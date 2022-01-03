use crate::routes::ConsoleAddr;
use axum_live_view::{html, Html, LiveView, live_view::Subscriptions, EventData, Updated, pubsub::Topic};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use axum::{async_trait, Json};

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

#[async_trait]
impl LiveView for ConnectionState {
    type Message = ();

    fn init(&self, setup: &mut Subscriptions<Self>) {
        setup.on(&msg_topic(self.stream_id), Self::msg);
    }

    async fn update(self, _: (), _: EventData) -> Updated<Self> {
        Updated::new(self)
    }

    fn render(&self) -> Html<()> {
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
    async fn msg(mut self, Json(msg): Json<Msg>) -> Updated<Self> {
        match msg {
            Msg::StreamEnded => self.state = State::StreamEnded,
            Msg::Error { code, message } => self.state = State::Error { code, message },
        }

        Updated::new(self)
    }
}

pub fn msg_topic(id: Uuid) -> impl Topic<Message = Json<Msg>> {
    axum_live_view::pubsub::topic(format!("connection-state/msg/{}", id))
}
