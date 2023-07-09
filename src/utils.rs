use std::io::{Read, Write};
use std::time::SystemTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PingMessage {
    pub sent_at: SystemTime,
    pub index: u32,
}

impl PingMessage {
    pub fn new() -> Self {
        PingMessage { sent_at: SystemTime::now(), index: 0 }
    }

    pub fn with_index(index: u32) -> Self {
        PingMessage {sent_at: SystemTime::now(), index}
    }
}

pub fn read_u32_le<R: Read>(source: &mut R) -> std::io::Result<u32> {
    let mut n = [0u8; 4];
    source.read_exact(&mut n)?;
    Ok(u32::from_le_bytes(n))
}

pub fn write_u32_le<W: Write>(target: &mut W, v: u32) -> std::io::Result<()> {
    let n: [u8; 4] = v.to_le_bytes();
    target.write_all(&n)
}