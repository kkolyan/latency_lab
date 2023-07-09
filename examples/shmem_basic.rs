use std::thread;

use shared_memory::*;


fn main() {
    let num_threads = 10;
    let count_to = 100;

    let mut threads = Vec::with_capacity(num_threads);
    let _ = std::fs::remove_file("basic_mapping");
    let max = count_to;
    // Spawn N threads
    for i in 0..num_threads {
        let thread_id = i + 1;
        threads.push(thread::spawn(move || {
            increment_value("basic_mapping", thread_id, max);
        }));
    }

    // Wait for threads to exit
    for t in threads.drain(..) {
        t.join().unwrap();
    }
}

/// Increments a value that lives in shared memory
fn increment_value(shmem_flink: &str, thread_num: usize, max: u8) {
    // Create or open the shared memory mapping
    let shmem = match ShmemConf::new().size(4096).flink(shmem_flink).create() {
        Ok(m) => m,
        Err(ShmemError::LinkExists) => ShmemConf::new().flink(shmem_flink).open().unwrap(),
        Err(e) => {
            eprintln!("Unable to create or open shmem flink {shmem_flink} : {e}");
            return;
        }
    };

    // Get pointer to the shared memory
    let raw_ptr = shmem.as_ptr();

    // WARNING: This is prone to race conditions as no sync/locking is used
    unsafe {
        while std::ptr::read_volatile(raw_ptr) < max {
            // Increment shared value by one
            *raw_ptr += 1;

            println!(
                "[thread:{}] {}",
                thread_num,
                std::ptr::read_volatile(raw_ptr)
            );

            // Sleep for a bit
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
}