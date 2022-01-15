use crate::routes::ConsoleAddr;
use axum::{
    async_trait,
    http::{HeaderMap, Uri},
};
use axum_live_view::{
    event_data::EventData,
    html,
    live_view::{Updated, ViewHandle},
    Html, LiveView,
};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct ConnectionState {
    state: State,
    addr: ConsoleAddr,
    rx: Option<mpsc::Receiver<Msg>>,
}

impl ConnectionState {
    pub fn new(addr: ConsoleAddr, rx: mpsc::Receiver<Msg>) -> Self {
        Self {
            state: State::Connecting,
            addr,
            rx: Some(rx),
        }
    }
}

#[async_trait]
impl LiveView for ConnectionState {
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
                    if handle.send(msg).await.is_err() {
                        break;
                    }
                }
                let _ = handle.send(Msg::StreamEnded).await;
            });
        }
        Ok(())
    }

    async fn update(
        mut self,
        msg: Self::Message,
        _: Option<EventData>,
    ) -> Result<Updated<Self>, Self::Error> {
        match msg {
            Msg::Connected => self.state = State::Connected,
            Msg::StreamEnded => self.state = State::StreamEnded,
            Msg::Error { code, message } => self.state = State::Error { code, message },
        }

        Ok(Updated::new(self))
    }

    fn render(&self) -> Html<Self::Message> {
        let connection_state = match &self.state {
            State::Connecting => html! {
                "Establishing connection to " { &self.addr.ip } ":" { &self.addr.port }
            },
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

#[derive(Clone, Debug)]
enum State {
    Connecting,
    Connected,
    StreamEnded,
    Error { code: i32, message: String },
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Msg {
    Connected,
    StreamEnded,
    Error { code: i32, message: String },
}
