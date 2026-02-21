// throughput/src/bin/writer.rs
use libc::*;
use std::ptr;
use std::sync::atomic::{fence, Ordering};
use throughput::{init_shared, Shared, BUF_SIZE};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args.len() > 3 {
        eprintln!("usage: {} <shm_name> [size_mb]", args[0]);
        std::process::exit(2);
    }

    let shm_name = &args[1];
    let total_bytes: u64 = args
        .get(2)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(100)
        * 1024
        * 1024;

    unsafe {
        // --- shm setup ---
        let name = std::ffi::CString::new(shm_name.as_str()).unwrap();
        let shm_size = std::mem::size_of::<Shared>();

        let fd = shm_open(name.as_ptr(), O_CREAT | O_RDWR, 0o666);
        if fd < 0 {
            panic!("shm_open: {:?}", std::io::Error::last_os_error());
        }
        if ftruncate(fd, shm_size as i64) != 0 {
            panic!("ftruncate: {:?}", std::io::Error::last_os_error());
        }

        let map = mmap(ptr::null_mut(), shm_size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        close(fd);
        if map == MAP_FAILED {
            panic!("mmap: {:?}", std::io::Error::last_os_error());
        }
        let shm = map as *mut Shared;

        // --- protocol init ---
        init_shared(shm, total_bytes);

        // --- writer loop (inline, simplest) ---
        let mut produced: u64 = 0;
        let local = vec![0xABu8; 1024 * 1024]; // 1 MiB constant chunk

        while produced < total_bytes {
            if (*shm).done.load(Ordering::Relaxed) < 0 {
                break;
            }

            // See latest reader progress before computing free space.
            let r = (*shm).read_pos.load(Ordering::Relaxed);
            fence(Ordering::Acquire);

            let w = (*shm).write_pos.load(Ordering::Relaxed);
            let used = w - r;

            if used as usize >= BUF_SIZE {
                std::hint::spin_loop();
                continue;
            }
            let free = (BUF_SIZE as u64) - used;

            let remaining = total_bytes - produced;
            let n = (remaining.min(free).min(local.len() as u64)) as usize;
            if n == 0 {
                std::hint::spin_loop();
                continue;
            }

            // Write into ring (handle wrap).
            let off = (w as usize) & (BUF_SIZE - 1);
            let first = n.min(BUF_SIZE - off);
            ptr::copy_nonoverlapping(local.as_ptr(), (*shm).buffer.as_mut_ptr().add(off), first);
            if first < n {
                ptr::copy_nonoverlapping(local.as_ptr().add(first), (*shm).buffer.as_mut_ptr(), n - first);
            }

            // Guarantee: data write before publishing new write_pos.
            fence(Ordering::Release);
            (*shm).write_pos.store(w + n as u64, Ordering::Relaxed);

            produced += n as u64;
        }

        fence(Ordering::Release);
        (*shm).done.store(1, Ordering::Relaxed);

        // --- cleanup ---
        munmap(map, shm_size);
        shm_unlink(name.as_ptr());
    }
}
