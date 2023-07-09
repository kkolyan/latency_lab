use raw_sync::Timeout::Infinite;
use latency_lab::shmem_lab::{ShmemReceiver, ShmemSender};
use latency_lab::utils::PingMessage;

fn main() {
    let sender: ShmemSender<PingMessage> = ShmemSender::open("hi");
    let receiver: ShmemReceiver<PingMessage> = ShmemReceiver::open("hi");

    loop {
        let message = receiver.receive(Infinite).unwrap();
        sender.send(message, Infinite).unwrap();
    }
}