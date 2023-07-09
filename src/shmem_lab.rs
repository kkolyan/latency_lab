

use std::marker::PhantomData;
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::SeqCst;
use std::thread::sleep;
use std::time::{Duration, SystemTime};

use raw_sync::Timeout;
use shared_memory::{Shmem, ShmemConf, ShmemError};

pub struct ShmemReceiver<T> {
    pd: PhantomData<T>,
    ctx: ShmemCtx,
}

struct ShmemCtx {
    shmem: Shmem,
    state: *mut AtomicU64,
    data: *mut u8,
}

const DEFAULT_SIZE: usize = 64 * 1024;

const STATE_READY_TO_WRITE: u64 = 2;
const STATE_READY_TO_READ: u64 = 3;

#[derive(Copy, Clone, Debug)]
pub enum ShmemLabError {
    Timeout
}

pub struct Success<T> {
    pub value: T,
    pub spin_count: u64,
}

impl<T> ShmemReceiver<T> {
    pub fn open(name: &str) -> ShmemReceiver<T> {
        ShmemReceiver { pd: Default::default(), ctx: open_shmem(name, DEFAULT_SIZE) }
    }

    pub fn receive(&self, timeout: Timeout) -> Result<Success<T>, ShmemLabError> {
        let timeout_at = match timeout {
            Timeout::Infinite => None,
            Timeout::Val(dur) => Some(SystemTime::now() + dur)
        };
        let state = unsafe { &mut *self.ctx.state };
        let mut spin_count: u64 = 0;
        while state.load(SeqCst) != STATE_READY_TO_READ {
            if let Some(at) = timeout_at {
                if SystemTime::now() > at {
                    return Err(ShmemLabError::Timeout);
                }
            }
            spin_count += 1;
        }
        let value = unsafe {
            (self.ctx.data as *mut T).read()
        };
        state.store(STATE_READY_TO_WRITE, SeqCst);
        Ok(Success { value, spin_count })
    }
}

pub struct ShmemSender<T> {
    pd: PhantomData<T>,
    ctx: ShmemCtx,
}

pub struct Stats {
    pub send_spins: Vec<u64>,
    pub receive_spins: Vec<u64>,
}

impl<T> ShmemSender<T> {
    pub fn open(name: &str) -> ShmemSender<T> {
        ShmemSender { pd: Default::default(), ctx: open_shmem(name, DEFAULT_SIZE) }
    }

    pub fn send(&self, value: T, timeout: Timeout) -> Result<Success<()>, ShmemLabError> {
        let timeout_at = match timeout {
            Timeout::Infinite => None,
            Timeout::Val(dur) => Some(SystemTime::now() + dur)
        };
        let state = unsafe { &mut *self.ctx.state };
        let mut spin_count = 0;
        while state.load(SeqCst) != STATE_READY_TO_WRITE {
            if let Some(at) = timeout_at {
                if SystemTime::now() > at {
                    return Err(ShmemLabError::Timeout);
                }
            }
            spin_count += 1;
        }
        *unsafe { &mut *(self.ctx.data as *mut T) } = value;
        state.store(STATE_READY_TO_READ, SeqCst);
        Ok(Success { value: (), spin_count })
    }
}

fn do_sleep(d: Duration) {
    let before = SystemTime::now();
    sleep(d);
    let spent = SystemTime::now().duration_since(before).unwrap();
    if spent > d * 10 {
        panic!("spent {:?} instead of {:?}", spent, d);
    }
}

fn open_shmem(name: &str, size: usize) -> ShmemCtx {
    let shmem = match ShmemConf::new().size(size).flink(name).create() {
        Ok(m) => m,
        Err(ShmemError::LinkExists) => ShmemConf::new().flink(name).open().unwrap(),
        Err(e) => {
            panic!("Unable to create or open shmem flink {} : {}", name, e)
        }
    };

    let mut data = shmem.as_ptr();
    let state: *mut AtomicU64;

    unsafe {
        state = &mut *(data as *mut AtomicU64);
        data = data.add(8);
    };
    loop {
        let state = unsafe { &mut *state };

        let state_before = state.load(SeqCst);
        if state_before == STATE_READY_TO_READ {
            break;
        }
        if state_before == STATE_READY_TO_WRITE {
            break;
        }
        if state.compare_exchange(state_before, STATE_READY_TO_WRITE, SeqCst, SeqCst).is_ok() {
            break;
        }
    }
    ShmemCtx {
        shmem,
        state,
        data,
    }
}