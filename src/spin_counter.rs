use std::cell::Cell;

struct SpinStats {
    prev_call_spin: Cell<u64>,
}

thread_local! {
    static SPIN_STATS: SpinStats = SpinStats {
        prev_call_spin: Cell::new(0)
    };
}

pub struct SpinCounter {
    value: u64,
}

impl SpinCounter {
    pub fn inc(&mut self) {
        self.value += 1;
    }

    pub fn new() -> Self {
        SpinCounter { value: 0 }
    }

    pub fn get(&self) -> u64 {
        self.value
    }

    pub fn get_last_result() -> u64 {
        SPIN_STATS.with(|it| it.prev_call_spin.get())
    }
}

impl Drop for SpinCounter {
    fn drop(&mut self) {
        SPIN_STATS.with(|it| it.prev_call_spin.set(self.value));
    }
}
