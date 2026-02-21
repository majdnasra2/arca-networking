// Process B': Opens existing shared memory, increments when even, times the benchmark
// Uses futex to sleep instead of busy spinning

use std::env;
use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

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
        std::ptr::null::<libc::timespec>(),
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
    let args: Vec<String> = env::args().collect();
    
    if args.len() < 3 {
        eprintln!("Usage: {} <shared_memory_name> <target_number>", args[0]);
        std::process::exit(1);
    }
    
    let shm_name = &args[1];
    let target: u32 = args[2].parse()
        .expect("Target must be a valid number");
    
    // Add '/' prefix if needed
    let shm_name = if shm_name.starts_with('/') {
        shm_name.to_string()
    } else {
        format!("/{}", shm_name)
    };
    
    // Convert to C string
    let c_name = CString::new(shm_name.as_bytes()).unwrap();
    
    // Open existing shared memory
    let fd = unsafe {
        libc::shm_open(
            c_name.as_ptr(),
            libc::O_RDWR,
            0o666
        )
    };
    
    if fd < 0 {
        panic!("Failed to open shared memory. Is process A running?");
    }
    
    // Map it into our address space
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
    
    // Cast to AtomicU32
    let shared = unsafe { &*(ptr as *const AtomicU32) };
    
    println!("Process B' ready. Target: {} (using futex)", target);
    
    let start = Instant::now();
    
    loop {
        let val = shared.load(Ordering::SeqCst);
        
        if val % 2 == 0 {
            // It's even, check if we reached target
            if val >= target {
                let elapsed = start.elapsed();
                
                println!("\nReached target: {}", val);
                println!("Total time: {:.3} ms", elapsed.as_secs_f64() * 1000.0);
                println!("Per handoff: {:.3} ns", elapsed.as_nanos() as f64 / target as f64);
                
                std::process::exit(0);
            }
            
            // Increment it
            shared.store(val + 1, Ordering::SeqCst);
            
            // Wake up process A if it's waiting
            unsafe {
                futex_wake(shared as *const AtomicU32, 1);
            }
        } else {
            // It's odd, wait for it to become even
            unsafe {
                futex_wait(shared as *const AtomicU32, val);
            }
            // After waking up, we loop again to check the new value
        }
    }
}
