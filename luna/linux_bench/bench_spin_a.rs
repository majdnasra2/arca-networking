// Process A: Creates shared memory, initializes to 0, increments when odd

use std::env;
use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};

fn main() {
    // Get shared memory name from command line
    // env::args() gives us the command line arguments
    // .nth(1) gets the second argument (first is program name)
    // .expect() crashes with a message if argument is missing
    let shm_name = env::args().nth(1)
        .expect("Usage: process_a <shared_memory_name>");
    
    // Add '/' prefix if not present (required for POSIX shared memory)
    let shm_name = if shm_name.starts_with('/') {
        shm_name
    } else {
        format!("/{}", shm_name)
    };
    
    // Convert Rust string to C string (null-terminated)
    let c_name = CString::new(shm_name.as_bytes()).unwrap();
    
    // Open/create shared memory
    let fd = unsafe {
        libc::shm_open(
            c_name.as_ptr(),           // pointer to name string
            libc::O_CREAT | libc::O_RDWR,  // create if needed, read/write
            0o666                       // permissions: read/write for all
        )
    };
    
    if fd < 0 {
        panic!("Failed to create shared memory");
    }
    
    // Set size to 4 bytes (size of u32)
    unsafe {
        libc::ftruncate(fd, 4);
    }
    
    // Map shared memory into our address space
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),      // let OS choose address
            4,                          // 4 bytes
            libc::PROT_READ | libc::PROT_WRITE,  // read and write access
            libc::MAP_SHARED,           // share with other processes
            fd,                         // our file descriptor
            0                           // offset 0
        )
    };
    
    if ptr == libc::MAP_FAILED {
        panic!("Failed to map shared memory");
    }
    
    // Cast raw pointer to AtomicU32 (atomic 32-bit unsigned integer)
    // &* converts pointer to reference
    let shared = unsafe { &*(ptr as *const AtomicU32) };
    
    shared.store(0, Ordering::SeqCst);
    println!("Process A ready. Waiting for odd numbers...");
    
    loop {
        let val = shared.load(Ordering::SeqCst);  
        if val % 2 == 1 {                          
            shared.store(val + 1, Ordering::SeqCst);  
        }
    }
}
