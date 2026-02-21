// throughput/src/bin/reader.rs
use libc::*;
use std::ptr;
use std::sync::atomic::{fence, Ordering};
use std::time::Instant;
use throughput::{wait_for_total_bytes, Shared, BUF_SIZE};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: {} <shm_name>", args[0]);
        std::process::exit(2);
    }

    let shm_name = &args[1];

    unsafe {
        // --- shm setup ---
        let name = std::ffi::CString::new(shm_name.as_str()).unwrap();
        let shm_size = std::mem::size_of::<Shared>();

        let fd = shm_open(name.as_ptr(), O_RDWR, 0o666);
        if fd < 0 {
            panic!("shm_open: {:?}", std::io::Error::last_os_error());
        }

        let map = mmap(ptr::null_mut(), shm_size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        close(fd);
        if map == MAP_FAILED {
            panic!("mmap: {:?}", std::io::Error::last_os_error());
        }
        let shm = map as *mut Shared;

        // --- wait for writer ---
        let total_bytes = wait_for_total_bytes(shm);

        // NOTE: allocates total_bytes bytes. Very large sizes (e.g., 4096MB) may OOM.
        let mut sink = vec![0u8; total_bytes as usize];

        // --- reader loop (inline, simplest) ---
        let start = Instant::now();

        let mut consumed: u64 = 0;
        let mut aborted = false;

        while consumed < total_bytes {
            // Observe writer progress; acquire fence makes buffer writes visible.
            let w = (*shm).write_pos.load(Ordering::Relaxed);
            fence(Ordering::Acquire);

            let r = (*shm).read_pos.load(Ordering::Relaxed);
            let avail = w.wrapping_sub(r);

            if avail == 0 {
                let done = (*shm).done.load(Ordering::Relaxed);
                fence(Ordering::Acquire);
                if done != 0 && consumed < total_bytes {
                    aborted = true;
                    break;
                }
                std::hint::spin_loop();
                continue;
            }

            let remaining = total_bytes - consumed;
            let n = (avail.min(remaining)) as usize;

            // Copy from ring into sink at offset consumed.
            let dst = sink.as_mut_ptr().add(consumed as usize);
            let off = (r as usize) & (BUF_SIZE - 1);
            let first = n.min(BUF_SIZE - off);
            ptr::copy_nonoverlapping((*shm).buffer.as_ptr().add(off), dst, first);
            if first < n {
                ptr::copy_nonoverlapping((*shm).buffer.as_ptr(), dst.add(first), n - first);
            }

            // Guarantee: we copied bytes out before publishing read_pos.
            fence(Ordering::Release);
            (*shm).read_pos.store(r + n as u64, Ordering::Relaxed);

            consumed += n as u64;
        }

        let elapsed = start.elapsed().as_secs_f64();

        if aborted {
            eprintln!("Aborted/ended early ({} bytes)", consumed);
        }

        if consumed > 0 && elapsed > 0.0 {
            let bytes = consumed as f64;
            let mib = bytes / (1024.0 * 1024.0);
            let mib_s = mib / elapsed;
            let gb_s = (bytes * 8.0) / elapsed / 1e9;
            println!(
                "Throughput: {:.2} MiB/s ({:.2} Gb/s), elapsed {:.3}s",
                mib_s, gb_s, elapsed
            );
        }

        // --- cleanup ---
        munmap(map, shm_size);
    }
}
