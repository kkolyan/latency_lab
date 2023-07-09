use std::time::SystemTime;

#[derive(Debug)]
pub struct PingMessage {
    pub sent_at: SystemTime,
}

impl PingMessage {
    pub fn new() -> Self {
        PingMessage { sent_at: SystemTime::now() }
    }
}