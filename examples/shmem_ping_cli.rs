use std::time::SystemTime;
use raw_sync::Timeout::Infinite;
use latency_lab::shmem_lab::{ShmemReceiver, ShmemSender};
use latency_lab::utils::PingMessage;

fn main() {

    let sender: ShmemSender<PingMessage> = ShmemSender::open("hi");
    let receiver: ShmemReceiver<PingMessage> = ShmemReceiver::open("hi");

    sender.send(PingMessage::new(), Infinite).unwrap();
    let response = receiver.receive(Infinite).unwrap();
    let lag = SystemTime::now().duration_since(response.sent_at).unwrap();
    println!("lag: {}", lag.as_nanos());
}