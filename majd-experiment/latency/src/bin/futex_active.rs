// Futex ping-pong but only count time spent active (increment + wake),
// not time blocked in futex_wait. Reports avg active latency = active_ns / iters.
//
// Coordination: check if even (parent's turn) / odd (child's turn); same as futex.
use libc::*;
use std::{mem, ptr};

const SHM_NAME: &str = "/pp_shm_futex_active";
const PAGE: usize = 4096;
const ITERS: u32 = 100_000;

#[repr(C)]
struct Shared {
    counter: i32,
    done: i32,
}

fn now_ns() -> u64 {
    unsafe {
        let mut ts: timespec = mem::zeroed();
        clock_gettime(CLOCK_MONOTONIC_RAW, &mut ts);
        (ts.tv_sec as u64) * 1_000_000_000 + ts.tv_nsec as u64
    }
}

unsafe fn futex_wait(addr: *const i32, expected: i32) {
    syscall(
        SYS_futex,
        addr,
        FUTEX_WAIT,
        expected,
        ptr::null::<timespec>(),
        ptr::null::<i32>(),
        0,
    );
}

unsafe fn futex_wake(addr: *const i32) {
    syscall(
        SYS_futex,
        addr,
        FUTEX_WAKE,
        1,
        ptr::null::<timespec>(),
        ptr::null::<i32>(),
        0,
    );
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

        (*shm).counter = 0;
        (*shm).done = 0;

        let pid = fork();
        if pid < 0 {
            panic!("fork: {}", std::io::Error::last_os_error());
        }

        if pid == 0 {
            // Child: check if odd (our turn); wait while even, then increment to even
            loop {
                while (*shm).counter % 2 == 0 {
                    futex_wait(&(*shm).counter as *const i32, (*shm).counter);
                }
                if (*shm).done != 0 {
                    break;
                }
                (*shm).counter += 1;
                futex_wake(&(*shm).counter as *const i32);
            }
            std::process::exit(0);
        }

        // Parent: check if even (our turn), time only increment+wake, then wait until even again
        let mut active_ns: u64 = 0;
        for _ in 0..ITERS {
            while (*shm).counter % 2 != 0 {
                futex_wait(&(*shm).counter as *const i32, (*shm).counter);
            }
            let t0 = now_ns();
            (*shm).counter += 1;
            futex_wake(&(*shm).counter as *const i32);
            let t1 = now_ns();
            active_ns += t1 - t0;

            while (*shm).counter % 2 != 0 {
                futex_wait(&(*shm).counter as *const i32, (*shm).counter);
            }
        }

        (*shm).done = 1;
        (*shm).counter += 1;
        futex_wake(&(*shm).counter as *const i32);

        let _ = waitpid(pid, ptr::null_mut(), 0);

        let avg_active_ns = active_ns / ITERS as u64;
        println!(
            "futex_active: avg active latency {} ns ({} round-trips, waiting time excluded)",
            avg_active_ns, ITERS
        );
    }
}
