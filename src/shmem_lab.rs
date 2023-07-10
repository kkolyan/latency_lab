use std::backtrace::Backtrace;
use std::io::{stdout, Write};
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::SeqCst;
use std::thread::sleep;
use std::time::{Duration, SystemTime};

use raw_sync::Timeout;
use shared_memory::{Shmem, ShmemConf, ShmemError};

const BACKTRACE_STUCK_SECS: f64 = 3.0;

pub struct ShmemReceiver<T> {
    ctx: ShmemCtx<T>,
}

pub struct ShmemCtx<T> {
    shmem: Shmem,
    state: *mut AtomicU64,
    pub data: *mut T,
    size: usize,
    invalid: bool,
}

const DEFAULT_SIZE: usize = 64 * 1024;

const STATE_ZERO: u64 = 0;
const STATE_READY_TO_WRITE: u64 = 1;
const STATE_WRITING: u64 = 2;
const STATE_READY_TO_READ: u64 = 3;
const STATE_READING: u64 = 4;

#[derive(Copy, Clone, Debug)]
pub enum ShmemLabError {
    Timeout,
    ConcurrentAccessDetected,
    Invalid,
}

pub struct SpinResult<T> {
    pub value: T,
    pub spin_count: u64,
}

impl<T: Copy> ShmemReceiver<T> {
    pub fn open(name: &str) -> ShmemReceiver<T> {
        ShmemReceiver { ctx: open_shmem(name) }
    }

    pub fn begin(&mut self, timeout: Timeout) -> SpinResult<Result<ShmemReceive<T>, ShmemLabError>> {
        if self.ctx.invalid {
            return SpinResult { value: Err(ShmemLabError::Invalid), spin_count: 0 };
        }
        let timeout_at = match timeout {
            Timeout::Infinite => None,
            Timeout::Val(dur) => Some(SystemTime::now() + dur)
        };
        let state = unsafe { &mut *self.ctx.state };
        let mut spin_count: u64 = 0;
        let mut backtrace_at = Some(SystemTime::now() + Duration::from_secs_f64(BACKTRACE_STUCK_SECS));
        loop {
            let curr_state = state.load(SeqCst);
            if curr_state == STATE_READY_TO_READ {
                break;
            }
            if let Some(at) = timeout_at {
                if SystemTime::now() > at {
                    return SpinResult {
                        value: Err(ShmemLabError::Timeout),
                        spin_count,
                    };
                }
            }

            if spin_count % 10000 == 0 {
                let backtrace = if let Some(backtrace_at) = &backtrace_at {
                    SystemTime::now() > *backtrace_at
                } else { false };
                if backtrace {
                    println!("stuck at {}:\n{}", curr_state, Backtrace::capture());
                    backtrace_at = None;
                }
            }

            spin_count += 1;
        }

        SpinResult {
            value: match state.compare_exchange(STATE_READY_TO_READ, STATE_READING, SeqCst, SeqCst) {
                Ok(_) => Ok(ShmemReceive { ctx: &mut self.ctx, closed: false }),
                Err(..) => Err(ShmemLabError::ConcurrentAccessDetected),
            },
            spin_count,
        }
    }
}

pub struct ShmemReceive<'a, T> {
    ctx: &'a mut ShmemCtx<T>,
    closed: bool,
}

impl<'a, T> ShmemReceive<'a, T> {
    pub fn end(mut self) -> Result<(), ShmemLabError> {
        if self.ctx.invalid {
            return Err(ShmemLabError::Invalid);
        }
        let state = unsafe { &mut *self.ctx.state };
        let result = state.compare_exchange(STATE_READING, STATE_READY_TO_WRITE, SeqCst, SeqCst);
        self.closed = true;
        match result {
            Err(_) => {
                self.ctx.invalid = true;
                Err(ShmemLabError::ConcurrentAccessDetected)
            }
            Ok(_) => Ok(()),
        }
    }
}

impl<'a, T> Deref for ShmemReceive<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.ctx.data }
    }
}

impl<'a, T> Drop for ShmemReceive<'a, T> {
    fn drop(&mut self) {
        if !self.closed {
            panic!("forgotten Sending handle detected");
        }
    }
}

pub struct ShmemSender<T> {
    ctx: ShmemCtx<T>,
}

pub struct ShmemSend<'a, T> {
    pub ctx: &'a mut ShmemCtx<T>,
    closed: bool,
}

impl<'a, T> ShmemSend<'a, T> {
    pub fn end(mut self) -> Result<(), ShmemLabError> {
        if self.ctx.invalid {
            return Err(ShmemLabError::Invalid);
        }
        let state = unsafe { &mut *self.ctx.state };
        let result = state.compare_exchange(STATE_WRITING, STATE_READY_TO_READ, SeqCst, SeqCst);
        self.closed = true;
        match result {
            Err(_) => {
                self.ctx.invalid = true;
                Err(ShmemLabError::ConcurrentAccessDetected)
            }
            Ok(..) => Ok(()),
        }
    }
}

impl<'a, T> Deref for ShmemSend<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.ctx.data }
    }
}

impl<'a, T> DerefMut for ShmemSend<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut (*self.ctx.data) }
    }
}

impl<'a, T> Drop for ShmemSend<'a, T> {
    fn drop(&mut self) {
        if !self.closed {
            panic!("forgotten Sending handle detected");
        }
    }
}

pub struct Stats {
    pub send_spins: Vec<u64>,
    pub receive_spins: Vec<u64>,
}

impl<T: Copy> ShmemSender<T> {
    pub fn open(name: &str) -> ShmemSender<T> {
        ShmemSender { ctx: open_shmem(name) }
    }

    pub fn begin(&mut self, timeout: Timeout) -> SpinResult<Result<ShmemSend<T>, ShmemLabError>> {
        if self.ctx.invalid {
            return SpinResult { value: Err(ShmemLabError::Invalid), spin_count: 0 };
        } else {
            let timeout_at = match timeout {
                Timeout::Infinite => None,
                Timeout::Val(dur) => Some(SystemTime::now() + dur)
            };
            let state = unsafe { &mut *self.ctx.state };
            let mut spin_count = 0;
            let mut backtrace_at = Some(SystemTime::now() + Duration::from_secs_f64(BACKTRACE_STUCK_SECS));
            loop {
                let curr_state = state.load(SeqCst);
                if curr_state == STATE_READY_TO_WRITE {
                    break;
                }
                if let Some(at) = timeout_at {
                    if SystemTime::now() > at {
                        return SpinResult {
                            value: Err(ShmemLabError::Timeout),
                            spin_count,
                        };
                    }
                }

                if spin_count % 10000 == 0 {
                    let backtrace = if let Some(backtrace_at) = &backtrace_at {
                        SystemTime::now() > *backtrace_at
                    } else { false };
                    if backtrace {
                        println!("stuck at {}:\n{}", curr_state, Backtrace::capture());
                        backtrace_at = None;
                    }
                }
                spin_count += 1;
            }
            SpinResult {
                value: match state.compare_exchange(STATE_READY_TO_WRITE, STATE_WRITING, SeqCst, SeqCst) {
                    Ok(_) => Ok(ShmemSend { ctx: &mut self.ctx, closed: false }),
                    Err(..) => Err(ShmemLabError::ConcurrentAccessDetected),
                },
                spin_count,
            }
        }
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

fn open_shmem<T: Copy>(name: &str) -> ShmemCtx<T> {
    let size = mem::size_of::<T>() + 8;
    let result = ShmemConf::new()
        .size(size)
        .flink(name)
        .create();
    let shmem = match result {
        Ok(m) => m,
        Err(ShmemError::LinkExists) => ShmemConf::new()
            .flink(name)
            .open()
            .unwrap(),
        Err(e) => {
            panic!("Unable to create or open shmem flink {} : {}", name, e)
        }
    };

    let data: *mut T;
    let state: *mut AtomicU64;

    unsafe {
        state = &mut *(shmem.as_ptr() as *mut AtomicU64);
        data = &mut *(shmem.as_ptr().add(8) as *mut T);
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
        if state_before != STATE_ZERO {
            panic!("shared memory is dirty");
        }
        if state.compare_exchange(state_before, STATE_READY_TO_WRITE, SeqCst, SeqCst).is_ok() {
            break;
        }
    }
    ShmemCtx {
        shmem,
        state,
        data,
        size,
        invalid: false,
    }
}