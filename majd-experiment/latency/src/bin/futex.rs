// Experiment (a): sleep/wake (futex). Fork one process; parent and child
// ping-pong on a single shared counter. Time total run, report avg latency
// per round-trip (total_ns / iters).
//
// Coordination: check if even / odd on the shared counter.
//   Even = parent's turn (parent waits until even, then increments to odd).
//   Odd  = child's turn  (child waits until odd,  then increments to even).
// One round-trip = parent sees even → increment → wait until even again.
use libc::*;
use std::{mem, ptr};

const SHM_NAME: &str = "/pp_shm_futex";
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

        // Parent: timed ping-pong; check if even (our turn), then wait until even again
        let t0 = now_ns();
        for _ in 0..ITERS {
            while (*shm).counter % 2 != 0 {
                futex_wait(&(*shm).counter as *const i32, (*shm).counter);
            }
            (*shm).counter += 1;
            futex_wake(&(*shm).counter as *const i32);

            while (*shm).counter % 2 != 0 {
                futex_wait(&(*shm).counter as *const i32, (*shm).counter);
            }
        }
        let t1 = now_ns();

        (*shm).done = 1;
        (*shm).counter += 1; // make odd so child wakes and sees done
        futex_wake(&(*shm).counter as *const i32);

        let _ = waitpid(pid, ptr::null_mut(), 0);

        let total_ns = t1 - t0;
        let avg_ns = total_ns / ITERS as u64;
        println!("futex: avg latency {} ns ({} round-trips)", avg_ns, ITERS);
    }
}
