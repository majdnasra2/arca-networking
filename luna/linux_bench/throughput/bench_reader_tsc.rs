use std::env;
use std::ffi::CString;
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering, fence};
use std::ptr;
use std::arch::x86_64::_rdtsc;

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
    
    println!("Reader: Waiting for writer to create shared memory...");
    
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
    
    println!("Reader: Shared memory found!");
    
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
    let mut dst = vec![0u8; transfer_size as usize];
    // Explicitly zero out dst in CHUNK_SIZE chunks before reader starts
    for chunk in dst.chunks_mut(CHUNK_SIZE as usize) {
        chunk.fill(0u8);
    }
    let mut total_read = 0u64;

    #[cfg(debug_assertions)]
    let mut xor_checksum: u8 = 0;

    // tsc
    let ckpt_total_interval = 10;
    let ckpt_interval_sz = (transfer_size + ckpt_total_interval - 1) / ckpt_total_interval;
    let mut ckpt_next = ckpt_interval_sz;

    // Change transfer_started to 1 (signal writer to start)
    transfer_started.store(1, Ordering::Release);
    println!("Reader: Signaled writer to start, waiting for data...");

    eprintln!("--- Reader checkpoint 0/{} tsc: {} ---", ckpt_total_interval, unsafe { _rdtsc() });
    
    // Main read loop
    while total_read < transfer_size {
        // Read indices
        let end_idx = end_index.load(Ordering::Acquire);
        let start_idx = start_index.load(Ordering::Acquire);
        
        // Calculate available length
        let avail_len = end_idx - start_idx;
        
        if avail_len > 0 {        
            let len = (CHUNK_SIZE as u64).min(transfer_size - total_read).min(avail_len);

            // Calculate read position with wrap-around
            let read_start = (start_idx % shm_size) as usize;
            let l = std::cmp::min(len, shm_size - read_start as u64) as usize;
            
            unsafe {
                // First part (until wrap or end of chunk)
                ptr::copy_nonoverlapping(
                    data_start.add(read_start),
                    dst.as_mut_ptr().add(total_read as usize),
                    l
                );
                
                // Second part (wrapped around to beginning)
                if l < len as usize {
                    ptr::copy_nonoverlapping(
                        data_start,
                        dst.as_mut_ptr().add(total_read as usize + l),
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

            if total_read > ckpt_next {
                eprintln!("--- Reader checkpoint {}/{} tsc: {} ---", ckpt_next / ckpt_interval_sz, 
                    ckpt_total_interval, unsafe { _rdtsc() });
                ckpt_next += ckpt_interval_sz;
            }
        } else {
            // Buffer empty, spin and wait
            std::hint::spin_loop();
        }
    }

    eprintln!("--- Reader checkpoint {}/{} tsc: {} ---", ckpt_next / ckpt_interval_sz, ckpt_total_interval, unsafe { _rdtsc() });
    println!("Reader: Finished reading {} bytes", total_read);

    transfer_started.store(0, Ordering::Relaxed);

    #[cfg(debug_assertions)]
    {
        // println!("{:?}", &dst[0..total_read as usize]);
        for i in 0..total_read as usize {
            xor_checksum ^= dst[i];
        }
        println!("Reader XOR checksum: 0x{:02X}", xor_checksum);
    }   
    
    // Cleanup
    unsafe {
        libc::munmap(ptr, total_size as usize);
        libc::close(fd);
        
        // Remove shared memory
        libc::shm_unlink(c_name.as_ptr());
    }
}
