use std::ops::{Deref, DerefMut};
use std::str::from_utf8;

use raw_sync::Timeout::Infinite;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::shmem_lab::{ShmemReceiver, ShmemSender, Stats, SpinResult};
use crate::utils::{read_u32_le, write_u32_le};

pub const MESSAGE_SHMEM_SIZE: usize = 64*512;

pub fn shmem_ping_send<T: Serialize>(m: &T, sender: &mut ShmemSender<[u8; MESSAGE_SHMEM_SIZE]>, stats: Option<&mut Stats>) {
    let json_string = serde_json::to_string(&m).unwrap();
    let json = json_string.as_bytes();

    let window = sender.begin(Infinite);
    let spin_count = window.spin_count;
    let mut window = window.value.unwrap();
    write_u32_le(&mut &mut window[0..4], json.len() as u32).unwrap();
    window[4..json.len() + 4].copy_from_slice(json);
    window.end().unwrap();
    if let Some(stats) = stats {
        stats.send_spins.push(spin_count);
    }
}

pub fn shmem_ping_receive<T: DeserializeOwned>(receiver: &mut ShmemReceiver<[u8; MESSAGE_SHMEM_SIZE]>, stats: Option<&mut Stats>) -> T {
    let window = receiver.begin(Infinite);
    let spin_count = window.spin_count;
    let window = window.value.unwrap();;
    let response = window.deref();
    if let Some(stats) = stats {
        stats.receive_spins.push(spin_count)
    }
    let n = read_u32_le(&mut &response[0..4]).unwrap();

    let end = n as usize + 4;
    let v: &[u8] = &response[4..(end)];
    let result = from_utf8(v).unwrap();
    let deserialized = serde_json::from_str(result).unwrap();
    window.end().unwrap();
    deserialized
}