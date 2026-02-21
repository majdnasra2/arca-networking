// throughput/src/lib.rs
// ONLY: shared layout + tiny helpers. No loops.

use std::hint::spin_loop;
use std::sync::atomic::{fence, AtomicI32, AtomicU64, Ordering};

pub const BUF_SIZE: usize = 4 * 1024 * 1024;

#[repr(C)]
pub struct Shared {
    pub total_bytes: AtomicU64, // 0 until writer publishes
    pub read_pos: AtomicU64,    // absolute counters
    pub write_pos: AtomicU64,
    pub done: AtomicI32,        // 0 running, 1 done, -1 aborted
    pub buffer: [u8; BUF_SIZE],
}

// Writer: init fields, then publish total_bytes.
// Fence makes init visible before total_bytes becomes nonzero.
pub unsafe fn init_shared(shm: *mut Shared, total_bytes: u64) {
    (*shm).read_pos.store(0, Ordering::Relaxed);
    (*shm).write_pos.store(0, Ordering::Relaxed);
    (*shm).done.store(0, Ordering::Relaxed);

    fence(Ordering::Release);
    (*shm).total_bytes.store(total_bytes, Ordering::Relaxed);
}

// Reader: wait until total_bytes published.
pub unsafe fn wait_for_total_bytes(shm: *mut Shared) -> u64 {
    loop {
        let tb = (*shm).total_bytes.load(Ordering::Relaxed);
        if tb != 0 {
            fence(Ordering::Acquire);
            return tb;
        }
        spin_loop();
    }
}
