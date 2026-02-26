use libc::*;
use std::ptr;
use std::sync::atomic::{fence, Ordering};
use std::time::Instant;
use throughput::{Shared, BUF_SIZE};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <shm_name>", args[0]);
        std::process::exit(2);
    }

    let shm_name = &args[1];
    let interval: u64 = 10_000_000; // Record every 10 million bytes
    let mut next_milestone = interval;
    let mut records = Vec::new();

    unsafe {
        let name = std::ffi::CString::new(shm_name.as_str()).unwrap();
        let shm_size = std::mem::size_of::<Shared>();
        let fd = shm_open(name.as_ptr(), O_RDWR, 0o666);
        if fd < 0 { panic!("SHM failed. Run writer first."); }

        let map = mmap(ptr::null_mut(), shm_size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        let shm = map as *mut Shared;

        while (*shm).total_bytes.load(Ordering::Relaxed) == 0 { std::hint::spin_loop(); }
        let total_bytes = (*shm).total_bytes.load(Ordering::Relaxed);
        let check_mode = (*shm).check_mode.load(Ordering::Relaxed) == 1;

        // PRE-ZERO the full sink to ensure no lazy allocation jitter
        let mut sink = vec![0u8; total_bytes as usize];
        sink.fill(0); 
        let sink_ptr = sink.as_mut_ptr();

        let mut running_xor: u8 = 0;
        
        // Timer starts right before signaling the writer
        let start = Instant::now();
        (*shm).start_signal.store(1, Ordering::Release);

        let mut consumed: u64 = 0;

        while consumed < total_bytes {
            let w = (*shm).write_pos.load(Ordering::Acquire);
            let r = (*shm).read_pos.load(Ordering::Relaxed);
            let avail = w.wrapping_sub(r);

            if avail == 0 {
                if (*shm).done.load(Ordering::Relaxed) != 0 { break; }
                std::hint::spin_loop();
                continue;
            }

            let n = avail.min(total_bytes - consumed) as usize;
            let off = (r as usize) & (BUF_SIZE - 1);
            let first = n.min(BUF_SIZE - off);

            ptr::copy_nonoverlapping((*shm).buffer.as_ptr().add(off), sink_ptr.add(consumed as usize), first);
            if first < n {
                ptr::copy_nonoverlapping((*shm).buffer.as_ptr(), sink_ptr.add(consumed as usize + first), n - first);
            }

            if check_mode {
                for i in 0..n {
                    running_xor ^= *sink_ptr.add(consumed as usize + i);
                }
            }

            fence(Ordering::Release);
            (*shm).read_pos.store(r + n as u64, Ordering::Relaxed);
            consumed += n as u64;

            // Log milestones every 10 million bytes
            while consumed >= next_milestone && next_milestone <= total_bytes {
                records.push((next_milestone, start.elapsed()));
                next_milestone += interval;
            }
        }

        let total_time = start.elapsed().as_secs_f64();

        // --- Final Report ---
        println!("\n{:<15} {:<15} {:<15}", "Bytes", "Time (s)", "Gb/s");
        for (b, t) in &records {
            let s = t.as_secs_f64();
            println!("{:<15} {:<15.6} {:<15.2}", b, s, (*b as f64 * 8.0) / (s * 1e9));
        }
        
        println!("{:-<45}", "");
        println!("{:<15} {:<15.6} {:<15.2} (TOTAL)", consumed, total_time, (consumed as f64 * 8.0) / (total_time * 1e9));

        if check_mode {
            let expected = (*shm).expected_xor.load(Ordering::Relaxed);
            if running_xor == expected {
                println!("✅ Verification Success (XOR {:#04x})", running_xor);
            } else {
                println!("❌ Verification Failed! Expected {:#04x}, got {:#04x}", expected, running_xor);
            }
        }
        
        munmap(map, shm_size);
    }
}