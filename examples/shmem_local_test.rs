use std::fs::remove_file;
use raw_sync::Timeout::Infinite;
use latency_lab::shmem_lab::{ShmemReceiver, ShmemSender};

// #[derive(Copy, Clone)]
// struct Message {
//     x: i32
// }

type Message = [u8; 512];

mod client {}

#[cfg(test)]
mod tests {
    use raw_sync::Timeout::Infinite;
    use latency_lab::shmem_lab::{ShmemReceiver, ShmemSender};
    use crate::{Message, Client, Server};

    #[test]
    fn it_works() {
        let mut server = Client::new();

        for i in 0..13 {
            server.tick(i);
        };

        println!("Done");
    }

    #[test]
    fn it_works2() {

        let mut server = Server::new();

        println!("Running...");
        loop {
            server.tick();
        }
    }
}

struct Client {
    sender: ShmemSender<Message>,
    receiver: ShmemReceiver<Message>,
}

impl Client {
    fn new() -> Client {
        Client {
            sender: ShmemSender::<Message>::open("shmem_local_test_up"),
            receiver: ShmemReceiver::<Message>::open("shmem_local_test_down"),
        }
    }

    fn tick(&mut self, i: i32) {
        let mut to_send = self.sender.begin(Infinite).value.unwrap();
        to_send[0] = i as u8;
        // to_send.x = i;
        to_send.end().unwrap();

        let mut received = self.receiver.begin(Infinite).value.unwrap();
        let x = received[0];
        // let x = received.x;
        received.end().unwrap();


        assert_eq!(i, x as i32)
    }
}

struct Server {
    sender: ShmemSender<Message>,
    receiver: ShmemReceiver<Message>,
}

impl Server {

    fn new() -> Server {
        remove_file("shmem_local_test_down");
        remove_file("shmem_local_test_up");
        Server {
            sender: ShmemSender::<Message>::open("shmem_local_test_down"),
            receiver: ShmemReceiver::<Message>::open("shmem_local_test_up"),
        }
    }
    fn tick(&mut self) {
        let received = self.receiver.begin(Infinite).value.unwrap();
        let x = received[0];
        // let x = received.x;
        received.end().unwrap();

        let mut to_send = self.sender.begin(Infinite).value.unwrap();
        to_send[0] = x;
        // to_send.x = x;
        to_send.end().unwrap();
    }
}

fn main() {
    remove_file("shmem_local_test_down");
    remove_file("shmem_local_test_up");
    // let mut server = Server::new();
    let mut sender = ShmemSender::<Message>::open("shmem_local_test_up");

    let mut to_send = sender.begin(Infinite).value.unwrap();
    let x = to_send.ctx.data;
    let xx= unsafe { &mut *x };
    xx[0] = 12u8;

    println!("Done");
}

