use raw_sync::Timeout::Infinite;
use latency_lab::shmem_lab::{ShmemReceiver, ShmemSender};
use latency_lab::shmem_ping::{MESSAGE_SHMEM_SIZE, shmem_ping_receive, shmem_ping_send};
use latency_lab::utils::PingMessage;

fn main() {
    let sender: ShmemSender<[u8; MESSAGE_SHMEM_SIZE]> = ShmemSender::open("shmem_ping_server_output");
    let receiver: ShmemReceiver<[u8; MESSAGE_SHMEM_SIZE]> = ShmemReceiver::open("shmem_ping_server_input");

    loop {
        let ping_message = shmem_ping_receive(&receiver);
        shmem_ping_send(&ping_message, &sender);
    }
}