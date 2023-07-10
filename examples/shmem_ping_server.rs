use std::fs::remove_file;
use raw_sync::Timeout::Infinite;
use latency_lab::shmem_lab::{ShmemReceiver, ShmemSender};
use latency_lab::shmem_ping::{MESSAGE_SHMEM_SIZE, shmem_ping_receive, shmem_ping_send};
use latency_lab::utils::PingMessage;

fn main() {
    remove_file("shmem_ping_server_input").unwrap();
    remove_file("shmem_ping_server_output").unwrap();
    let mut sender: ShmemSender<[u8; MESSAGE_SHMEM_SIZE]> = ShmemSender::open("shmem_ping_server_output");
    let mut receiver: ShmemReceiver<[u8; MESSAGE_SHMEM_SIZE]> = ShmemReceiver::open("shmem_ping_server_input");

    loop {
        let ping_message: PingMessage = shmem_ping_receive(&mut receiver, None);
        shmem_ping_send(&ping_message, &mut sender, None);
    }
}