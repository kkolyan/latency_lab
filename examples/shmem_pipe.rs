use raw_sync::Timeout::Infinite;
use latency_lab::{ShmemReceiver, ShmemSender};
use latency_lab::shmem_lab::*;
use latency_lab::utils::PingMessage;

fn main() {
    let sender: ShmemSender<PingMessage> = ShmemSender::open("hi");
    let receiver: ShmemReceiver<PingMessage> = ShmemReceiver::open("hi");
    sender.send(PingMessage::new(), Infinite).unwrap();
    let m = receiver.receive(Infinite).unwrap();
    println!("{:?}", m);
}