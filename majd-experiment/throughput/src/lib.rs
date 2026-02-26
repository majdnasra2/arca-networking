use std::sync::atomic::{AtomicI32, AtomicU64, AtomicU8, Ordering, fence};

pub const BUF_SIZE: usize = 4 * 1024 * 1024;

#[repr(C)]
pub struct Shared {
    pub total_bytes: AtomicU64,
    pub read_pos: AtomicU64,
    pub write_pos: AtomicU64,
    pub done: AtomicI32,
    pub start_signal: AtomicI32,
    pub check_mode: AtomicI32,
    pub expected_xor: AtomicU8,
    pub buffer: [u8; BUF_SIZE],
}

pub unsafe fn init_shared(shm: *mut Shared, total_bytes: u64, check_mode: bool) {
    (*shm).read_pos.store(0, Ordering::Relaxed);
    (*shm).write_pos.store(0, Ordering::Relaxed);
    (*shm).done.store(0, Ordering::Relaxed);
    (*shm).start_signal.store(0, Ordering::Relaxed);
    (*shm).expected_xor.store(0, Ordering::Relaxed);
    (*shm).check_mode.store(if check_mode { 1 } else { 0 }, Ordering::Relaxed);
    
    fence(Ordering::Release);
    (*shm).total_bytes.store(total_bytes, Ordering::Relaxed);
}