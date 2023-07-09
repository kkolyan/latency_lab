use raw_sync::Timeout::Infinite;
use latency_lab::shmem_lab::*;
use latency_lab::utils::PingMessage;

fn main() {
    let sender: ShmemSender<PingMessage> = ShmemSender::open("shmem_ping_server_input");
    let receiver: ShmemReceiver<PingMessage> = ShmemReceiver::open("shmem_ping_server_input");
    sender.send(PingMessage::new(), Infinite).unwrap();
    let m = receiver.receive(Infinite).unwrap();
    println!("{:?}", m);
}