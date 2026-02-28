use std::arch::x86_64::{_mm_lfence, _mm_mfence, _rdtsc};

#[inline]
pub fn read_tsc() -> u64 {
    unsafe {
        _mm_mfence();
        _mm_lfence();
        let tsc = _rdtsc();
        _mm_lfence();
        tsc
    }
}
