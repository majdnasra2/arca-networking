// Process A': Creates shared memory, initializes to 0, increments when odd
// Uses futex to sleep instead of busy spinning

use std::env;
use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};

// Futex operations
const FUTEX_WAIT: i32 = 0;
const FUTEX_WAKE: i32 = 1;

// Wrapper for futex system call
unsafe fn futex_wait(addr: *const AtomicU32, expected: u32) -> i32 {
    libc::syscall(
        libc::SYS_futex,
        addr,
        FUTEX_WAIT,
        expected,
        std::ptr::null::<libc::timespec>(),  // no timeout
        std::ptr::null::<u32>(),
        0
    ) as i32
}

unsafe fn futex_wake(addr: *const AtomicU32, num_to_wake: i32) -> i32 {
    libc::syscall(
        libc::SYS_futex,
        addr,
        FUTEX_WAKE,
        num_to_wake,
        std::ptr::null::<libc::timespec>(),
        std::ptr::null::<u32>(),
        0
    ) as i32
}

fn main() {
    // Get shared memory name from command line
    let shm_name = env::args().nth(1)
        .expect("Usage: process_a_futex <shared_memory_name>");
    
    // Add '/' prefix if not present
    let shm_name = if shm_name.starts_with('/') {
        shm_name
    } else {
        format!("/{}", shm_name)
    };
    
    // Convert Rust string to C string
    let c_name = CString::new(shm_name.as_bytes()).unwrap();
    
    // Open/create shared memory
    let fd = unsafe {
        libc::shm_open(
            c_name.as_ptr(),
            libc::O_CREAT | libc::O_RDWR,
            0o666
        )
    };
    
    if fd < 0 {
        panic!("Failed to create shared memory");
    }
    
    // Set size to 4 bytes
    unsafe {
        libc::ftruncate(fd, 4);
    }
    
    // Map shared memory into our address space
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            4,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0
        )
    };
    
    if ptr == libc::MAP_FAILED {
        panic!("Failed to map shared memory");
    }
    
    // Cast raw pointer to AtomicU32
    let shared = unsafe { &*(ptr as *const AtomicU32) };
    
    // Initialize to 0
    shared.store(0, Ordering::SeqCst);
    println!("Process A' ready. Waiting for odd numbers (using futex)...");
    
    loop {
        let val = shared.load(Ordering::SeqCst);
        
        if val % 2 == 1 {
            // It's odd, increment it
            shared.store(val + 1, Ordering::SeqCst);
            
            // Wake up process B if it's waiting
            unsafe {
                futex_wake(shared as *const AtomicU32, 1);
            }
        } else {
            // It's even, wait for it to become odd
            // futex_wait will return if the value changes from val
            unsafe {
                futex_wait(shared as *const AtomicU32, val);
            }
            // After waking up, we loop again to check the new value
        }
    }
}
