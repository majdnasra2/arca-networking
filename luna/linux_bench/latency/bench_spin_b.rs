// Process B: Opens existing shared memory, increments when even, times the benchmark

use std::env;
use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

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
    
    // Open existing shared memory (no O_CREAT flag this time)
    let fd = unsafe {
        libc::shm_open(
            c_name.as_ptr(),
            libc::O_RDWR,     // just read/write, don't create
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
    
    println!("Process B ready. Target: {}", target);
    
    let start = Instant::now();
    
    loop {
        let val = shared.load(Ordering::SeqCst);
        
        if val % 2 == 0 {       
            if val >= target {
                let elapsed = start.elapsed();
                
                println!("\nReached target: {}", val);
                println!("Total time: {:.3} ms", elapsed.as_secs_f64() * 1000.0);
                println!("Per handoff: {:.3} ns", elapsed.as_nanos() as f64 / target as f64);
                
                std::process::exit(0);
            }
            shared.store(val + 1, Ordering::SeqCst);
        }
    }
}

// 4KB, 2MB, 1GB
