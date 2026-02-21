// Experiment (b): busy loop. Fork one process; parent and child ping-pong
// on a single shared counter by spinning (no sleep). Time total run,
// report avg latency per round-trip (total_ns / iters).
//
// Coordination: check if even / odd on the shared counter.
//   Even = parent's turn; odd = child's turn. Same protocol as futex.
use libc::*;
use std::ptr;
use std::sync::atomic::{AtomicI32, Ordering};

const SHM_NAME: &str = "/pp_shm_busy";
const PAGE: usize = 4096;
const ITERS: u32 = 100_000;

#[repr(C)]
struct Shared {
    counter: AtomicI32,
    done: AtomicI32,
}

fn now_ns() -> u64 {
    unsafe {
        let mut ts: libc::timespec = std::mem::zeroed();
        libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut ts);
        (ts.tv_sec as u64) * 1_000_000_000 + ts.tv_nsec as u64
    }
}

fn main() {
    unsafe {
        let name = std::ffi::CString::new(SHM_NAME).unwrap();
        let fd = shm_open(name.as_ptr(), O_CREAT | O_RDWR, 0o666);
        if fd < 0 {
            panic!("shm_open: {}", std::io::Error::last_os_error());
        }
        if ftruncate(fd, PAGE as i64) != 0 {
            panic!("ftruncate: {}", std::io::Error::last_os_error());
        }
        let map = mmap(
            ptr::null_mut(),
            PAGE,
            PROT_READ | PROT_WRITE,
            MAP_SHARED,
            fd,
            0,
        );
        if map == MAP_FAILED {
            panic!("mmap: {}", std::io::Error::last_os_error());
        }
        let shm = map as *mut Shared;

        (*shm).counter.store(0, Ordering::SeqCst);
        (*shm).done.store(0, Ordering::SeqCst);

        let pid = fork();
        if pid < 0 {
            panic!("fork: {}", std::io::Error::last_os_error());
        }

        if pid == 0 {
            // Child: check if odd (our turn); spin while even, then increment to even
            loop {
                while (*shm).counter.load(Ordering::SeqCst) % 2 == 0 {
                    if (*shm).done.load(Ordering::SeqCst) != 0 {
                        std::process::exit(0);
                    }
                    std::hint::spin_loop();
                }
                if (*shm).done.load(Ordering::SeqCst) != 0 {
                    break;
                }
                (*shm).counter.fetch_add(1, Ordering::SeqCst);
            }
            std::process::exit(0);
        }

        // Parent: check if even (our turn), increment to odd, then wait until even again
        let t0 = now_ns();
        for _ in 0..ITERS {
            while (*shm).counter.load(Ordering::SeqCst) % 2 != 0 {
                std::hint::spin_loop();
            }
            (*shm).counter.fetch_add(1, Ordering::SeqCst);

            while (*shm).counter.load(Ordering::SeqCst) % 2 != 0 {
                std::hint::spin_loop();
            }
        }
        let t1 = now_ns();

        (*shm).done.store(1, Ordering::SeqCst);

        let _ = waitpid(pid, ptr::null_mut(), 0);

        let total_ns = t1 - t0;
        let avg_ns = total_ns / ITERS as u64;
        println!("busy:  avg latency {} ns ({} round-trips)", avg_ns, ITERS);
    }
}
