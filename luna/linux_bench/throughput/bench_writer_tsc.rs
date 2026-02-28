use std::env;
use std::ffi::CString;
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering, fence};
use std::time::Instant;
use std::ptr;
mod tsc;
// use rand::RngCore;

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
    
    // Total size: 8 bytes (start_index) + 8 bytes (end_index) + 4 bytes (transfer_started) + shm_size (data)
    let total_size = 20 + shm_size;
    
    // Set size
    unsafe {
        libc::ftruncate(fd, total_size as i64);
    }
    
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
    
    let base = ptr as *mut u8;
    let start_index = unsafe { &*(base as *mut AtomicU64) };
    let end_index = unsafe { &*(base.add(8) as *mut AtomicU64) };
    let transfer_started = unsafe { &*(base.add(16) as *mut AtomicU32) };
    let data_start = unsafe { base.add(20) };
    
    // Prepare data chunk (all zeros)
    // let src = vec![0u8; CHUNK_SIZE as usize];

    // Fill with pattern: 1, 2, 3, ..., 255, 1, 2, 3, ...
    let mut src = vec![0u8; CHUNK_SIZE as usize];
    for i in 0..CHUNK_SIZE as usize {
        src[i] = ((i % 255) + 1) as u8;
    }
    // rand::thread_rng().fill_bytes(&mut src);

    let mut total_written = 0u64;

    #[cfg(debug_assertions)]
    let mut xor_checksum: u8 = 0;

    // tsc
    let ckpt_total_interval = 10;
    let ckpt_interval_sz = (transfer_size + ckpt_total_interval - 1) / ckpt_total_interval;
    let mut ckpt_next = ckpt_interval_sz;
    
    println!("Writer: Waiting for reader to start (transfer_started=1)...");
    
    // Wait till reader changes transfer_started to 1
    while transfer_started.load(Ordering::Acquire) == 0 {
        std::hint::spin_loop();
    }
    
    println!("Writer: Reader ready, starting write...");
    let start_time = Instant::now();
    eprintln!("--- Writer checkpoint 0/{} tsc: {} ---", ckpt_total_interval, tsc::read_tsc());
    
    // Main write loop
    while total_written < transfer_size {
        // Read indices
        let end_idx = end_index.load(Ordering::Acquire);
        let start_idx = start_index.load(Ordering::Acquire);
        
        // Calculate unused length
        let unused_len = shm_size - (end_idx - start_idx);
        
        if unused_len > 0 {            
            let len = (CHUNK_SIZE as u64).min(transfer_size - total_written).min(unused_len);
            
            // Calculate write position with wrap-around
            let write_start = (end_idx % shm_size) as usize;
            let l = std::cmp::min(len, shm_size - write_start as u64) as usize;
            
            unsafe {
                // First part (until wrap or end of chunk)
                ptr::copy_nonoverlapping(
                    src.as_ptr(),
                    data_start.add(write_start),
                    l
                );
                
                // Second part (wrapped around to beginning)
                if l < len as usize {
                    ptr::copy_nonoverlapping(
                        src.as_ptr().add(l),
                        data_start,
                        len as usize - l
                    );
                }
            }
            
            // Barrier: smp_wmb() - ensure data writes complete before index update
            // On x86, this is just a compiler barrier since Store→Store is guaranteed
            fence(Ordering::Release);
            
            // Update end_index
            end_index.store(end_idx + len, Ordering::Release);
            total_written += len;

            #[cfg(debug_assertions)]
            {
                // println!("{:?}", &src[0..len as usize]);
                for i in 0..len as usize {
                    xor_checksum ^= src[i];
                }
            }

            if total_written > ckpt_next {
                eprintln!("--- Writer checkpoint {}/{} tsc: {} ---", ckpt_next / ckpt_interval_sz, 
                    ckpt_total_interval, tsc::read_tsc());
                ckpt_next += ckpt_interval_sz;
            }
            
        } else {
            // Buffer full, spin and wait
            std::hint::spin_loop();
        }
    }

    eprintln!("--- Writer checkpoint {}/{} tsc: {} ---", ckpt_next / ckpt_interval_sz, ckpt_total_interval, tsc::read_tsc());
    println!("Writer: Finished writing {} bytes", total_written);
    
    #[cfg(debug_assertions)]
    println!("Writer XOR checksum: 0x{:02X}", xor_checksum);

    println!("Writer: Waiting for reader to finish ...");
    
    // Wait till reader changes transfer_started to 0
    while transfer_started.load(Ordering::Relaxed) != 0 {
        std::hint::spin_loop();
    }
    
    let elapsed = start_time.elapsed();
    
    println!("========================================");
    println!("WRITER STATS");
    println!("========================================");
    // println!("Total time: {:.6} seconds", elapsed.as_secs_f64());
    println!("Total time: {} µs, {} s", elapsed.as_micros(), elapsed.as_secs_f64());
    println!("Data written: {} bytes", total_written );
    println!("Throughput: {:.4} GB / s", total_written as f64 / (1024.0 * 1024.0 * 1024.0 * elapsed.as_secs_f64()));
    println!("========================================");
    
    // Cleanup
    unsafe {
        libc::munmap(ptr, total_size as usize);
        libc::close(fd);
    }
}
