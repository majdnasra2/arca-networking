use libc::*;
use std::ptr;
use std::sync::atomic::{fence, Ordering};
use throughput::{init_shared, Shared, BUF_SIZE};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <shm_name> [size_mb] [--check]", args[0]);
        std::process::exit(2);
    }

    let shm_name = &args[1];
    let total_bytes: u64 = args.get(2)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(100) * 1024 * 1024;
    let check_mode = args.contains(&"--check".to_string());

    unsafe {
        let name = std::ffi::CString::new(shm_name.as_str()).unwrap();
        let shm_size = std::mem::size_of::<Shared>();

        let fd = shm_open(name.as_ptr(), O_CREAT | O_RDWR, 0o666);
        ftruncate(fd, shm_size as i64);
        let map = mmap(ptr::null_mut(), shm_size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        let shm = map as *mut Shared;

        init_shared(shm, total_bytes, check_mode);

        println!("Writer ready. Waiting for Reader signal...");
        while (*shm).start_signal.load(Ordering::Acquire) == 0 {
            std::hint::spin_loop();
        }

        let mut produced: u64 = 0;
        let mut running_xor: u8 = 0;
        let local_data = vec![0xABu8; 1024 * 1024]; 

        while produced < total_bytes {
            let r = (*shm).read_pos.load(Ordering::Acquire);
            let w = (*shm).write_pos.load(Ordering::Relaxed);
            let used = w.wrapping_sub(r);

            if used as usize >= BUF_SIZE {
                std::hint::spin_loop();
                continue;
            }

            let n = ((BUF_SIZE as u64 - used).min(total_bytes - produced).min(local_data.len() as u64)) as usize;
            
            if check_mode {
                for i in 0..n {
                    running_xor ^= ((produced + i as u64) % 256) as u8;
                }
            }

            let off = (w as usize) & (BUF_SIZE - 1);
            let first = n.min(BUF_SIZE - off);
            ptr::copy_nonoverlapping(local_data.as_ptr(), (*shm).buffer.as_mut_ptr().add(off), first);
            if first < n {
                ptr::copy_nonoverlapping(local_data.as_ptr(), (*shm).buffer.as_mut_ptr(), n - first);
            }

            fence(Ordering::Release);
            (*shm).write_pos.store(w + n as u64, Ordering::Relaxed);
            produced += n as u64;
        }

        if check_mode { (*shm).expected_xor.store(running_xor, Ordering::Relaxed); }
        (*shm).done.store(1, Ordering::Release);
        munmap(map, shm_size);
    }
}