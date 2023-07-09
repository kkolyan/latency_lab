use std::io::{BufReader, BufWriter, Read, stdin, Stdin, stdout, Stdout, Write};
use std::time::SystemTime;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
struct Message {
    index: u32,
    sent: SystemTime,
}

fn main() {
    let mut stdin = BufReader::new(stdin());
    let mut stdout = BufWriter::new(stdout());
    let flush = true;
    loop {
        let result = read_message(&mut stdin);
        if let Ok(m) = result {
            if write_message(&mut stdout, &m).is_err() || (flush && stdout.flush().is_err()) {
                break
            }
        }
    }
}

fn read_message<R: Read>(mut server_socket: &mut R) -> Result<Message, String> {
    let n = read_u32_le(&mut server_socket)
        .map_err(|e| e.to_string())?;
    serde_json::from_reader::<_, Message>(server_socket.take(n.into()))
        .map_err(|e| e.to_string())
}

fn write_message<W: Write>(mut server_socket: &mut W, m: &Message) -> std::io::Result<()> {
    let json = serde_json::to_string(&m).unwrap();
    write_u32_le(&mut server_socket, json.len() as u32);
    server_socket.write_all(json.as_bytes())
}

fn read_u32_le<R: Read>(source: &mut R) -> std::io::Result<u32> {
    let mut n = [0u8; 4];
    source.read_exact(&mut n)?;
    Ok(u32::from_le_bytes(n))
}

fn write_u32_le<W: Write>(target: &mut W, v: u32) -> std::io::Result<()> {
    let n: [u8; 4] = v.to_le_bytes();
    target.write_all(&n)
}