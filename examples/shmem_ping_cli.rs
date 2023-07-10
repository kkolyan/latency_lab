use std::time::SystemTime;
use latency_lab::shmem_lab::{ShmemReceiver, ShmemSender};
use latency_lab::shmem_ping::{MESSAGE_SHMEM_SIZE, shmem_ping_receive, shmem_ping_send};
use latency_lab::utils::PingMessage;

fn main() {

    let mut sender: ShmemSender<[u8; MESSAGE_SHMEM_SIZE]> = ShmemSender::open("shmem_ping_server_input");
    let mut receiver: ShmemReceiver<[u8; MESSAGE_SHMEM_SIZE]> = ShmemReceiver::open("shmem_ping_server_output");

    let original = PingMessage::new();
    shmem_ping_send(&original, &mut sender, None);
    let response: PingMessage = shmem_ping_receive(&mut receiver, None);
    let lag = SystemTime::now().duration_since(response.sent_at).unwrap();
    println!("lag: {}", lag.as_nanos());
    assert_eq!(original.index, response.index);
}