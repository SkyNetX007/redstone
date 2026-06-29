// redstone-tui/src/event.rs
use crossterm::event::{KeyEvent, MouseEvent};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum Event {
    Tick,
    Key(KeyEvent),
    Resize(u16, u16),
    Mouse(MouseEvent),
    DaemonMessage {
        profile: String,
        line: String,
    },
    DaemonConnected {
        profile: String,
    },
    StartServer {
        profile: String,
    },
    SlpResult {
        profile: String,
        result: Result<redstone_core::slp::ServerStatus, String>,
    },
}

pub struct EventLoop {
    rx: mpsc::Receiver<Event>,
    tx: mpsc::Sender<Event>,
}

impl EventLoop {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::channel::<Event>(256);
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tick_rate);
            loop {
                interval.tick().await;
                if tx_clone.send(Event::Tick).await.is_err() {
                    break;
                }
            }
        });
        let tx_ev = tx.clone();
        std::thread::spawn(move || {
            loop {
                match crossterm::event::read() {
                    Ok(crossterm::event::Event::Key(key)) => {
                        let _ = tx_ev.blocking_send(Event::Key(key));
                    }
                    Ok(crossterm::event::Event::Resize(w, h)) => {
                        let _ = tx_ev.blocking_send(Event::Resize(w, h));
                    }
                    Ok(crossterm::event::Event::Mouse(ev)) => {
                        let _ = tx_ev.blocking_send(Event::Mouse(ev));
                    }
                    Err(_) => break,
                    _ => {}
                }
            }
        });
        Self { rx, tx }
    }

    pub async fn recv(&mut self) -> Option<Event> {
        self.rx.recv().await
    }

    pub fn sender(&self) -> mpsc::Sender<Event> {
        self.tx.clone()
    }
}
