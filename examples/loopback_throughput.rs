extern crate core;

use std::hint::spin_loop;
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::channel;
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};
use raw_sync::Timeout::Infinite;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thousands::Separable;
use latency_lab::shmem_lab::{ShmemReceiver, ShmemSender, Stats};
use latency_lab::shmem_ping::{shmem_ping_receive, shmem_ping_send};
use latency_lab::utils::{PingMessage, read_u32_le, write_u32_le};

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
struct Message {
    // index: u32,
    sent_at: SystemTime,
}

fn main() {
    println!("Example: {}", serde_json::to_string(&Message { sent_at: SystemTime::now() }).unwrap());
    let batch_client = Duration::from_secs_f64(0.001);
    let batch_server = Duration::from_secs_f64(0.001);
    let buffering = Buffering::of(Some(batch_client), Some(batch_server));
    let iterations = 12000;
    //{"sent_at":{"secs_since_epoch":1688901474,"nanos_since_epoch":490919000}}
    test("Just ser+deser  ", iterations, || PingLocalSerde);
    test("shmem in-process", iterations, || ShmemSerdePing::new_in_process("loopback_thoughput_shmem_in_process"));
    test("shmem x-process ", iterations, || ShmemSerdePing::new("shmem_ping_server_input", "shmem_ping_server_output"));
    // test("JSON + loopback async direct        ", iterations, || PingLoopbackSerde::new(true, false, None));
    // test("JSON + loopback async buffered      ", iterations, || PingLoopbackSerde::new(false, false, None));
    test("buffered unlim  ", iterations, || PingLoopbackSerde::new(buffering, true, None, transport_loopback()));
    test("buffered 20000/s", iterations, || PingLoopbackSerde::new(buffering, true, Some(20000.0), transport_loopback()));
    test("buffered 10000/s", iterations, || PingLoopbackSerde::new(buffering, true, Some(10000.0), transport_loopback()));
    test("buffered 5000/s ", iterations, || PingLoopbackSerde::new(buffering, true, Some(5000.0), transport_loopback()));
    test("buffered 2000/s ", iterations, || PingLoopbackSerde::new(buffering, true, Some(2000.0), transport_loopback()));
    // test("JSON + loopback buffered 200/s ", iterations, || PingLoopbackSerde::new(false, true, Some(200.0)));
    test("direct unlim    ", iterations, || PingLoopbackSerde::new(Buffering::empty(), true, None, transport_loopback()));
    test("direct 20000/s  ", iterations, || PingLoopbackSerde::new(Buffering::empty(), true, Some(20000.0), transport_loopback()));
    test("direct 10000/s  ", iterations, || PingLoopbackSerde::new(Buffering::empty(), true, Some(10000.0), transport_loopback()));
    test("direct 5000/s   ", iterations, || PingLoopbackSerde::new(Buffering::empty(), true, Some(5000.0), transport_loopback()));
    test("direct 2000/s   ", iterations, || PingLoopbackSerde::new(Buffering::empty(), true, Some(2000.0), transport_loopback()));
}

fn test<T: Ping, F: FnOnce() -> T>(name: &str, iterations: u32, factory: F) {
    let mut stat = Vec::with_capacity(iterations as usize);
    let mut t = factory();

    let before = SystemTime::now();

    t.do_job(&mut stat, iterations as usize);

    let total = SystemTime::now().duration_since(before).unwrap();

    stat.sort();
    print_stats(name, &stat, total, iterations as f64 / (total.as_secs_f64()));
}

fn print_stats(name: &str, stat: &Vec<Duration>, total: Duration, message_per_sec: f64) {
    let min = stat.iter().min().unwrap();
    let max = stat.iter().max().unwrap();
    let sum: Duration = stat.iter().sum();
    let avg = sum / stat.len() as u32;
    let med = stat.get(stat.len() / 2).unwrap();
    let pct95 = stat.get(((stat.len() as f64) * 0.95) as usize).unwrap();
    let pct05 = stat.get(((stat.len() as f64) * 0.05) as usize).unwrap();
    println!(
        "{}: {: >12}..{: >15} | avg: {: >12} | med: {: >12} | pct-95: {: >14} | pct-05: {: >12} | rate/s: {:.2}",
        name,
        min.as_nanos().separate_with_commas(),
        max.as_nanos().separate_with_commas(),
        avg.as_nanos().separate_with_commas(),
        med.as_nanos().separate_with_commas(),
        pct95.as_nanos().separate_with_commas(),
        pct05.as_nanos().separate_with_commas(),
        message_per_sec
    );
}

trait Ping {
    fn do_job(&mut self, stats: &mut Vec<Duration>, iterations: usize);
}

type Trait = dyn Fn(Message) -> Message;

#[derive(Copy, Clone)]
struct Buffering {
    client: Option<Duration>,
    server: Option<Duration>,
}

impl Buffering {
    fn empty() -> Self {
        Buffering { client: None, server: None }
    }

    fn of(client: Option<Duration>, server: Option<Duration>) -> Self {
        Buffering { client, server }
    }
}

struct PingLocalSerde;

impl Ping for PingLocalSerde {
    fn do_job(&mut self, stats: &mut Vec<Duration>, iterations: usize) {
        for i in 0..iterations {
            let m = Message { sent_at: SystemTime::now() };
            let json = serde_json::to_string(&m).unwrap();
            let m: Message = serde_json::from_str(json.as_str()).unwrap();
            stats.push(SystemTime::now().duration_since(m.sent_at).unwrap());
        }
    }
}

struct PingLoopbackSerde<R, W> {
    transport: Transport<R, W>,
    flush: Buffering,
    sync: bool,
    produce_per_sec: Option<f64>,
}

struct Transport<R, W> {
    client_socket: (R, W),
    server_socket: (R, W),
}

#[derive(Clone)]
enum Payload {
    Length(u32, String),
    Text(String),
}

enum WriteNext {
    Length(Box<[u8]>),
    Text(Box<[u8]>),
}

trait MyClone {
    fn my_clone(&self) -> Self;
}

impl MyClone for TcpStream {
    fn my_clone(&self) -> Self {
        self.try_clone().unwrap()
    }
}

impl<R: MyClone, W: MyClone> MyClone for (R, W) {
    fn my_clone(&self) -> Self {
        let (r, w) = self;
        (r.my_clone(), w.my_clone())
    }
}

impl<R: Read + MyClone, W: Write + MyClone> MyClone for Transport<R, W> {
    fn my_clone(&self) -> Self {
        Transport {
            client_socket: self.client_socket.my_clone(),
            server_socket: self.server_socket.my_clone(),
        }
    }
}

fn transport_loopback() -> Transport<TcpStream, TcpStream> {
    let addr = "localhost:8080";
    let server_instance = TcpListener::bind(addr).unwrap();

    let client_socket = spawn("connect", move || {
        TcpStream::connect(addr).unwrap()
    });

    let server_socket = {
        let server_instance = server_instance.try_clone().unwrap();
        spawn("accept", move || {
            server_instance.accept().unwrap().0
        })
    };

    let client_socket = client_socket.join().unwrap();
    let server_socket = server_socket.join().unwrap();

    Transport {
        client_socket: (client_socket.my_clone(), client_socket),
        server_socket: (server_socket.my_clone(), server_socket),
    }
}

impl<R: Read + MyClone + Send + 'static, W: Write + MyClone + Send + 'static> PingLoopbackSerde<R, W> {
    fn new(flush: Buffering, sync: bool, produce_per_sec: Option<f64>, transport: Transport<R, W>) -> Self {
        if !sync && produce_per_sec.is_some() {
            todo!();
        }

        let (reader, writer) = &transport.server_socket;

        if let Some(buffering) = flush.server {
            let (sender, receiver) = channel::<Message>();
            {
                let mut server_socket = reader.my_clone();
                spawn("pump reader", move || {
                    loop {
                        let result = read_message(&mut server_socket);
                        if let Ok(m) = result {
                            sender.send(m).unwrap();
                        }
                    }
                });
            }
            {
                let mut server_socket = writer.my_clone();
                spawn("pump writer", move || {
                    let mut batch = Vec::new();
                    let mut next_flush = SystemTime::now() + buffering;
                    loop {
                        for m in receiver.try_iter() {
                            batch.push(m);
                        }
                        let now = SystemTime::now();
                        if now >= next_flush {
                            next_flush = now + buffering;
                            for m in batch.iter() {
                                if write_message(&mut server_socket, m).is_err() || server_socket.flush().is_err() {
                                    // println!("thread pump writer finished job");
                                    return;
                                }
                            }
                            batch.clear();
                        }
                    }
                });
            }
        } else {
            let mut reader = reader.my_clone();
            let mut writer = writer.my_clone();
            spawn("pump", move || {
                loop {
                    let result = read_message(&mut reader);
                    if let Ok(m) = result {
                        if write_message(&mut writer, &m).is_err() || writer.flush().is_err() {
                            println!("thread pump finished job");
                            break;
                        }
                    }
                }
            });
        }

        PingLoopbackSerde {
            transport,
            flush,
            sync,
            produce_per_sec,
        }
    }

    fn do_job_sync(&mut self, stats: &mut Vec<Duration>, iterations: usize) {
        let mut client_socket = BufReader::new(self.transport.client_socket.0.my_clone());
        let responses = spawn("read responses", move || {
            let mut stats: Vec<Duration> = Vec::new();
            let buf = &mut [0u8; 64 * 1024];
            let mut rem = iterations;
            while rem > 0 {
                let n = read_u32_le(&mut client_socket).unwrap();
                let body = &mut buf[0..(n as usize)];
                client_socket.read_exact(body).unwrap();
                let m: Message = serde_json::from_slice(body).unwrap();
                stats.push(SystemTime::now().duration_since(m.sent_at).unwrap());
                rem -= 1;
            }
            stats
        });

        let mut rem = iterations;
        let mut writer = BufWriter::new(&mut self.transport.client_socket.1);
        let sleep_secs = self.produce_per_sec.map(|it| 1.0 / it);
        let mut next_flush = self.flush.client.map(|it| SystemTime::now() + it);
        while rem > 0 {
            let m = Message { sent_at: SystemTime::now() };
            let string = serde_json::to_string(&m).unwrap();
            let data = string.as_bytes();
            write_u32_le(&mut writer, data.len() as u32).unwrap();
            writer.write_all(data).unwrap();
            if let Some(delay) = self.flush.client {
                let now = SystemTime::now();
                if now > next_flush.unwrap() {
                    writer.flush().unwrap();
                    next_flush = Some(now + delay)
                }
            } else {
                writer.flush().unwrap();
            }
            rem -= 1;
            if let Some(sleep_secs) = sleep_secs {
                sleep_spin(Duration::from_secs_f64(sleep_secs));
            }
        }
        writer.flush().unwrap();


        *stats = responses.join().unwrap();
    }

    fn do_job_async(&mut self, stats: &mut Vec<Duration>, iterations: usize) {
        todo!()
        // let buffer: &mut [u8] = &mut [0u8; 65 * 1024];
        //
        // let mut reads_rem = iterations;
        // let mut writes_rem = iterations;
        // let mut read_next_length: Option<u32> = None;
        // let mut write_next: Option<WriteNext> = None;
        //
        // while reads_rem > 0 || writes_rem > 0 {
        //     'read:
        //     while reads_rem > 0 {
        //         match read_next_length {
        //             Some(n) => {
        //                 let buf = &mut buffer[0..(n as usize)];
        //                 match self.client_socket.read_exact(buf) {
        //                     Ok(_) => {
        //                         let m: Message = serde_json::from_slice(buf).unwrap_or_else(|_| panic!("failed to parse {}", from_utf8(buf).unwrap()));
        //                         stats.push(SystemTime::now().duration_since(m.sent_at).unwrap());
        //                         reads_rem -= 1;
        //                         read_next_length = None;
        //                     }
        //                     Err(e) => {
        //                         if e.kind() == ErrorKind::WouldBlock {
        //                             break 'read;
        //                         }
        //                         panic!("read body error: {}", e);
        //                     }
        //                 };
        //             }
        //             None => {
        //                 match read_u32_le(&mut self.client_socket) {
        //                     Ok(n) => read_next_length = Some(n),
        //                     Err(e) => {
        //                         if e.kind() == ErrorKind::WouldBlock {
        //                             break 'read;
        //                         }
        //                         panic!("read length error: {}", e);
        //                     }
        //                 };
        //             }
        //         }
        //     }
        //
        //     'write:
        //     while writes_rem > 0 {
        //         write_next = match write_next.take() {
        //             None => {
        //                 let m = Message { sent_at: SystemTime::now() };
        //                 let json = serde_json::to_string(&m).unwrap();
        //                 Some(WriteNext::Length(json.into_boxed_str().into_boxed_bytes()))
        //             }
        //             Some(it) => match it {
        //                 WriteNext::Length(s) => {
        //                     match write_u32_le(&mut self.client_socket, s.len() as u32) {
        //                         Ok(_) => {
        //                             Some(WriteNext::Text(s))
        //                         }
        //                         Err(e) => {
        //                             if e.kind() == ErrorKind::WouldBlock {
        //                                 write_next = Some(WriteNext::Length(s));
        //                                 break 'write;
        //                             }
        //                             panic!("write length error: {}", e);
        //                         }
        //                     }
        //                 }
        //                 WriteNext::Text(s) => {
        //                     match self.client_socket.write_all(s.deref()) {
        //                         Ok(_) => {
        //                             if self.flush {
        //                                 self.client_socket.flush();
        //                             }
        //                             writes_rem -= 1;
        //                             None
        //                         }
        //                         Err(e) => {
        //                             if e.kind() == ErrorKind::WouldBlock {
        //                                 write_next = Some(WriteNext::Text(s));
        //                                 break 'write;
        //                             }
        //                             panic!("write body error: {}", e);
        //                         }
        //                     }
        //                 }
        //             }
        //         };
        //     }
        // }
    }
}

fn sleep_spin(duration: Duration) {
    let before = SystemTime::now();
    while SystemTime::now().duration_since(before).unwrap() < duration {
        spin_loop();
    }
}

fn spawn<T, F>(name: &str, f: F) -> JoinHandle<T>
    where F: FnOnce() -> T,
          F: Send + 'static,
          T: Send + 'static {
    thread::Builder::new().name(name.into()).spawn(f).unwrap()
}

impl<R: Read + MyClone + Send + 'static, W: Write + MyClone + Send + 'static> Ping for PingLoopbackSerde<R, W> {
    fn do_job(&mut self, stats: &mut Vec<Duration>, iterations: usize) {
        if self.sync {
            self.do_job_sync(stats, iterations);
        } else {
            self.do_job_async(stats, iterations);
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

const MESSAGE_SHMEM_SIZE: usize = 512;

struct ShmemSerdePing {
    sender: ShmemSender<[u8; MESSAGE_SHMEM_SIZE]>,
    receiver: ShmemReceiver<[u8; MESSAGE_SHMEM_SIZE]>,
}

impl ShmemSerdePing {
    fn new_in_process(name: &str) -> ShmemSerdePing {
        ShmemSerdePing {
            sender: ShmemSender::open(name),
            receiver: ShmemReceiver::open(name),
        }
    }
    fn new(send: &str, receive: &str) -> ShmemSerdePing {
        ShmemSerdePing {
            sender: ShmemSender::open(send),
            receiver: ShmemReceiver::open(receive),
        }
    }
}


impl Ping for ShmemSerdePing {
    fn do_job(&mut self, stats: &mut Vec<Duration>, iterations: usize) {
        let mut shmem_stats = Stats { send_spins: vec![], receive_spins: vec![] };
        for i in 0..iterations {
            shmem_ping_send(&PingMessage::new(), &self.sender, Some(&mut shmem_stats));
            let response = shmem_ping_receive(&self.receiver, Some(&mut shmem_stats));
            stats.push(SystemTime::now().duration_since(response.sent_at).unwrap());
        }

        print_spin_stats("send", &mut shmem_stats.send_spins);
        print_spin_stats("receive", &mut shmem_stats.receive_spins);
    }
}

fn print_spin_stats(kind: &str, spins: &mut Vec<u64>) {
    spins.sort();
    let sum: u64 = spins.iter().sum();
    println!(
        "↓ spins. type: {}. {}..{} avg: {}, med: {}, pct95: {}",
        kind,
        spins.first().unwrap(),
        spins.last().unwrap(),
        sum / spins.len() as u64,
        spins.get(spins.len() / 2).unwrap(),
        spins.get((spins.len() as f64 * 0.95) as usize).unwrap(),
    )
}