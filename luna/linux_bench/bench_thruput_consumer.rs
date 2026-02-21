use std::env;
use std::ffi::CString;
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering, fence};
use std::ptr;

const CHUNK_SIZE: u32 = 1024;

fn main() {
    let args: Vec<String> = env::args().collect();
    
    if args.len() < 4 {
        eprintln!("Usage: {} <shared_mem_name> <share_mem_size> <transfer_size>", args[0]);
        std::process::exit(1);
    }
    
    let shm_name = &args[1];
    let shm_size: u64 = args[2].parse()
        .expect("share_mem_size must be a valid number");
    let transfer_size: u64 = args[3].parse()
        .expect("transfer_size must be a valid number");
    
    // Add '/' prefix if needed
    let shm_name = if shm_name.starts_with('/') {
        shm_name.to_string()
    } else {
        format!("/{}", shm_name)
    };
    
    // Convert to C string
    let c_name = CString::new(shm_name.as_bytes()).unwrap();
    
    println!("Consumer: Waiting for producer to create shared memory...");
    
    // Open existing shared memory (no O_CREAT flag)
    let fd = loop {
        let fd = unsafe {
            libc::shm_open(
                c_name.as_ptr(),
                libc::O_RDWR,
                0o666
            )
        };
        
        if fd >= 0 {
            break fd;
        }
        
        // Wait a bit and retry
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    
    println!("Consumer: Shared memory found!");
    
    // Total size: 8 bytes (start_index) + 8 bytes (end_index) + 4 bytes (transfer_started) + shm_size (data)
    let total_size = 20 + shm_size;
    
    // Map shared memory into our address space
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            total_size as usize,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0
        )
    };
    
    if ptr == libc::MAP_FAILED {
        panic!("Failed to map shared memory");
    }
    
    // Get pointers to shared variables
    let base = ptr as *mut u8;
    let start_index = unsafe { &*(base as *mut AtomicU64) };
    let end_index = unsafe { &*(base.add(8) as *mut AtomicU64) };
    let transfer_started = unsafe { &*(base.add(16) as *mut AtomicU32) };
    let data_start = unsafe { base.add(20) };
    
    // Prepare buffer for reading
    let mut dst = vec![0u8; CHUNK_SIZE as usize];
    let mut total_read = 0u64;
    
    // Change transfer_started to 1 (signal producer to start)
    transfer_started.store(1, Ordering::Release);
    
    println!("Consumer: Signaled producer to start, waiting for data...");
    
    // Main read loop
    while total_read < transfer_size {
        // Read indices
        let end_idx = end_index.load(Ordering::Acquire);
        let start_idx = start_index.load(Ordering::Acquire);
        
        // Calculate available length
        let avail_len = end_idx - start_idx;
        
        if avail_len > 0 {                        
            let len = std::cmp::min(CHUNK_SIZE as u64, avail_len);
            
            // Calculate read position with wrap-around
            let read_start = (start_idx % shm_size) as usize;
            let l = std::cmp::min(len, shm_size - (start_idx % shm_size)) as usize;
            
            unsafe {
                // First part (until wrap or end of chunk)
                ptr::copy_nonoverlapping(
                    data_start.add(read_start),
                    dst.as_mut_ptr(),
                    l
                );
                
                // Second part (wrapped around to beginning)
                if l < len as usize {
                    ptr::copy_nonoverlapping(
                        data_start,
                        dst.as_mut_ptr().add(l),
                        len as usize - l
                    );
                }
            }
            
            // Barrier: smp_wmb() - ensure data reads complete before index update
            // On x86, this is just a compiler barrier since Store→Store is guaranteed
            fence(Ordering::Release);
            
            // Update start_index
            start_index.store(start_idx + len, Ordering::Relaxed);
            total_read += len;

            // println!("{:?}", &dst[0..CHUNK_SIZE as usize]);
            
        } else {
            // Buffer empty, spin and wait
            std::hint::spin_loop();
        }
    }
    
    println!("Consumer: Finished reading {} bytes", total_read);
    
    // Change transfer_started to 0 (signal producer we're done)
    transfer_started.store(0, Ordering::Relaxed);
    
    // Cleanup
    unsafe {
        libc::munmap(ptr, total_size as usize);
        libc::close(fd);
        
        // Remove shared memory
        libc::shm_unlink(c_name.as_ptr());
    }
}
