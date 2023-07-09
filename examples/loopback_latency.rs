extern crate core;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};
use raw_sync::Timeout::Infinite;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use latency_lab::shmem_lab::{ShmemReceiver, ShmemSender};

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
struct Message {
    index: u32,
    sent: SystemTime,
}

fn main() {
    let iterations = 10000;
    test("json            ", iterations, || PingLocalSerde);
    // test("Ping loopback serde json w/flush ", iterations, || PingLoopbackSerde::new(true));
    test("wo/flush", iterations, || PingLoopbackSerde::new(false));
    // test("pipes   ", iterations, || PingStdPipesSerde::new());
    test("shmem   ", iterations, || ShmemSerdePing::new("loopback_latency.shmem".to_string()));
}

fn test<T: Ping, F: FnOnce() -> T>(name: &str, iterations: usize, factory: F) {
    let mut stat = Vec::with_capacity(iterations);
    let mut t = factory();
    for i in 0..iterations {
        let index = i as u32;
        let result = t.ping(Message { sent: SystemTime::now(), index });
        assert_eq!(result.index, index);
        let lag = SystemTime::now().duration_since(result.sent).unwrap();
        stat.push(lag);
    }
    stat.sort();
    let min = stat.iter().min().unwrap();
    let max = stat.iter().max().unwrap();
    let sum: Duration = stat.iter().sum();
    let avg = sum / stat.len() as u32;
    let med = stat.get(stat.len() / 2).unwrap();
    let pct95 = stat.get(((stat.len() as f64) * 0.95) as usize).unwrap();
    let pct05 = stat.get(((stat.len() as f64) * 0.05) as usize).unwrap();
    println!("Test \"{}\": {}..{}, avg: {}, med: {}, pct-95: {}, pct-05: {}", name, min.as_nanos(), max.as_nanos(), avg.as_nanos(), med.as_nanos(), pct95.as_nanos(), pct05.as_nanos());
}

trait Ping {
    fn ping(&mut self, m: Message) -> Message;
}

type Trait = dyn Fn(Message) -> Message;

struct PingLocalSerde;

impl Ping for PingLocalSerde {
    fn ping(&mut self, m: Message) -> Message {
        let json = serde_json::to_string(&m).unwrap();
        serde_json::from_str(json.as_str()).unwrap()
    }
}

struct PingLoopbackSerde {
    client_socket: TcpStream,
    server_socket: TcpStream,
    server_instance: TcpListener,
    flush: bool,
}

impl PingLoopbackSerde {
    fn new(flush: bool) -> Self {
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

        {
            let mut server_socket = server_socket.try_clone().unwrap();
            spawn("pump", move || {
                loop {
                    let result = read_message(&mut server_socket);
                    if let Ok(m) = result {
                        if write_message(&mut server_socket, &m).is_err() || (flush && server_socket.flush().is_err()) {
                            println!("thread pump finished job");
                            break;
                        }
                    }
                }
            });
        }

        PingLoopbackSerde {
            client_socket,
            server_socket,
            server_instance,
            flush,
        }
    }
}

fn spawn<T, F>(name: &str, f: F) -> JoinHandle<T>
    where F: FnOnce() -> T,
          F: Send + 'static,
          T: Send + 'static {
    thread::Builder::new().name(name.into()).spawn(f).unwrap()
}

impl Ping for PingLoopbackSerde {
    fn ping(&mut self, m: Message) -> Message {
        write_message(&mut self.client_socket, &m).unwrap();
        if self.flush {
            self.client_socket.flush();
        }
        read_message(&mut self.client_socket).unwrap()
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

struct PingStdPipesSerde {
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl PingStdPipesSerde {
    fn new() -> Self {
        let std::process::Child { stdin, stdout, .. } = match Command::new("C:/dev/rust/reactex/latency_lab/target/debug/examples/ping_cli.exe")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn() {
            Err(why) => panic!("couldn't spawn wc: {}", why),
            Ok(process) => process,
        };

        PingStdPipesSerde { stdin: stdin.unwrap(), stdout: stdout.unwrap() }
    }
}

impl Ping for PingStdPipesSerde {
    fn ping(&mut self, m: Message) -> Message {
        write_message(&mut self.stdin, &m).unwrap();
        read_message(&mut self.stdout).unwrap()
    }
}

fn read_u32_le<R: Read>(source: &mut R) -> std::io::Result<u32> {
    let mut n = [0u8; 4];
    source.read_exact(&mut n)?;
    Ok(u32::from_le_bytes(n))
}

fn write_u32_le<W: Write>(target: &mut W, v: u32) {
    let n: [u8; 4] = v.to_le_bytes();
    target.write_all(&n).unwrap();
}

const MESSAGE_SHMEM_SIZE: usize = 512;

struct ShmemSerdePing {
    sender: ShmemSender<[u8; MESSAGE_SHMEM_SIZE]>,
    receiver: ShmemReceiver<[u8; MESSAGE_SHMEM_SIZE]>,
}

impl ShmemSerdePing {
    fn new(name: String) -> ShmemSerdePing {
        ShmemSerdePing {
            sender: ShmemSender::open(name.as_str()),
            receiver: ShmemReceiver::open(name.as_str()),
        }
    }
}

impl Ping for ShmemSerdePing {
    fn ping(&mut self, m: Message) -> Message {
        let mut data = [0u8; MESSAGE_SHMEM_SIZE];
        let json_string = serde_json::to_string(&m).unwrap();
        let json = json_string.as_bytes();
        write_u32_le(&mut &mut data[0..4], json.len() as u32);
        data[4..json.len() + 4].copy_from_slice(json);
        self.sender.send(data, Infinite).unwrap();
        let response = self.receiver.receive(Infinite).unwrap();
        let n = read_u32_le(&mut &response[0..4]).unwrap();

        serde_json::from_slice(&response[4..(n as usize + 4)]).unwrap()
    }
}