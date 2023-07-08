extern crate core;

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::ops::Deref;
use std::str::from_utf8;
use std::sync::mpsc::channel;
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
struct Message {
    // index: u32,
    sent_at: SystemTime,
}

fn main() {
    let iterations = 10000;
    test("Ping local serde json                 ", iterations, || PingLocalSerde);
    test("Ping loopback serde json w/flush      ", iterations, || PingLoopbackSerde::new(true, false));
    test("Ping loopback serde json wo/flush     ", iterations, || PingLoopbackSerde::new(false, false));
    test("Ping loopback serde json wo/flush sync", iterations, || PingLoopbackSerde::new(false, true));
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
    println!("Test \"{}\": {}..{}, avg: {}, med: {}, pct-95: {}, pct-05: {}, total: {}, rate/s: {}", name, min.as_nanos(), max.as_nanos(), avg.as_nanos(), med.as_nanos(), pct95.as_nanos(), pct05.as_nanos(), total.as_nanos(), message_per_sec);
}

trait Ping {
    fn do_job(&mut self, stats: &mut Vec<Duration>, iterations: usize);
}

type Trait = dyn Fn(Message) -> Message;

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

struct PingLoopbackSerde {
    client_socket: TcpStream,
    server_socket: TcpStream,
    server_instance: TcpListener,
    flush: bool,
    sync: bool,
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

impl PingLoopbackSerde {
    fn new(flush: bool, sync: bool) -> Self {
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
                    let result = Self::read_message(&mut server_socket);
                    if let Ok(m) = result {
                        if Self::write_message(&mut server_socket, &m).is_err() || (flush && server_socket.flush().is_err()) {
                            println!("thread pump finished job");
                            break;
                        }
                    }
                }
            });
        }
        client_socket.set_nonblocking(!sync).unwrap();

        PingLoopbackSerde {
            client_socket,
            server_socket,
            server_instance,
            flush,
            sync,
        }
    }

    fn do_job_sync(&mut self, stats: &mut Vec<Duration>, iterations: usize) {
        let mut client_socket = self.client_socket.try_clone().unwrap();
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
        while rem > 0 {
            let m = Message { sent_at: SystemTime::now() };
            let string = serde_json::to_string(&m).unwrap();
            let data = string.as_bytes();
            write_u32_le(&mut self.client_socket, data.len() as u32).unwrap();
            self.client_socket.write_all(data).unwrap();
            rem -= 1;
        }


        *stats = responses.join().unwrap();
    }

    fn do_job_async(&mut self, stats: &mut Vec<Duration>, iterations: usize) {
        let buffer: &mut [u8] = &mut [0u8; 65 * 1024];

        let mut reads_rem = iterations;
        let mut writes_rem = iterations;
        let mut read_next_length: Option<u32> = None;
        let mut write_next: Option<WriteNext> = None;

        while reads_rem > 0 || writes_rem > 0 {
            'read:
            while reads_rem > 0 {
                match read_next_length {
                    Some(n) => {
                        let buf = &mut buffer[0..(n as usize)];
                        match self.client_socket.read_exact(buf) {
                            Ok(_) => {
                                let m: Message = serde_json::from_slice(buf).unwrap_or_else(|_| panic!("failed to parse {}", from_utf8(buf).unwrap()));
                                stats.push(SystemTime::now().duration_since(m.sent_at).unwrap());
                                reads_rem -= 1;
                                read_next_length = None;
                            }
                            Err(e) => {
                                if e.kind() == ErrorKind::WouldBlock {
                                    break 'read;
                                }
                                panic!("read body error: {}", e);
                            }
                        };
                    }
                    None => {
                        match read_u32_le(&mut self.client_socket) {
                            Ok(n) => read_next_length = Some(n),
                            Err(e) => {
                                if e.kind() == ErrorKind::WouldBlock {
                                    break 'read;
                                }
                                panic!("read length error: {}", e);
                            }
                        };
                    }
                }
            }

            'write:
            while writes_rem > 0 {
                write_next = match write_next.take() {
                    None => {
                        let m = Message { sent_at: SystemTime::now() };
                        let json = serde_json::to_string(&m).unwrap();
                        Some(WriteNext::Length(json.into_boxed_str().into_boxed_bytes()))
                    }
                    Some(it) => match it {
                        WriteNext::Length(s) => {
                            match write_u32_le(&mut self.client_socket, s.len() as u32) {
                                Ok(_) => {
                                    Some(WriteNext::Text(s))
                                }
                                Err(e) => {
                                    if e.kind() == ErrorKind::WouldBlock {
                                        write_next = Some(WriteNext::Length(s));
                                        break 'write;
                                    }
                                    panic!("write length error: {}", e);
                                }
                            }
                        }
                        WriteNext::Text(s) => {
                            match self.client_socket.write_all(s.deref()) {
                                Ok(_) => {
                                    if self.flush {
                                        self.client_socket.flush();
                                    }
                                    writes_rem -= 1;
                                    None
                                }
                                Err(e) => {
                                    if e.kind() == ErrorKind::WouldBlock {
                                        write_next = Some(WriteNext::Text(s));
                                        break 'write;
                                    }
                                    panic!("write body error: {}", e);
                                }
                            }
                        }
                    }
                };
            }
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
    fn do_job(&mut self, stats: &mut Vec<Duration>, iterations: usize) {
        if self.sync {
            self.do_job_sync(stats, iterations);
        } else {
            self.do_job_async(stats, iterations);
        }
    }
}

impl PingLoopbackSerde {
    fn read_message(mut server_socket: &mut TcpStream) -> Result<Message, String> {
        let n = read_u32_le(&mut server_socket)
            .map_err(|e| e.to_string())?;
        serde_json::from_reader::<_, Message>(server_socket.take(n.into()))
            .map_err(|e| e.to_string())
    }

    fn write_message(mut server_socket: &mut TcpStream, m: &Message) -> std::io::Result<()> {
        let json = serde_json::to_string(&m).unwrap();
        write_u32_le(&mut server_socket, json.len() as u32);
        server_socket.write_all(json.as_bytes())
    }
}

fn read_u32_le<R: Read>(source: &mut R) -> std::io::Result<u32> {
    let mut n = [0u8; 4];
    source.read_exact(&mut n)?;
    Ok(u32::from_le_bytes(n))
}

fn write_u32_le<W: Write>(target: &mut W, v: u32) -> std::io::Result<()> {
    let n: [u8; 4] = v.to_le_bytes();
    target.write_all(&n)
}