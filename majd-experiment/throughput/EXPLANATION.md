# Throughput Experiment — Code Explanation for Review

This document explains what the code does, what we use, and how we know it is correct and safe. It is written so it can be used to explain the project to a professor or reviewer.

---

## 1. What the project does

We measure **throughput** between two processes using **shared memory**:

- **Writer process**: Fills a fixed-size circular buffer in shared memory with data (zeros) and advances a “write position.”
- **Reader process**: Reads from the same buffer into its own memory and advances a “read position,” then reports MiB/s and Gb/s.

There is **exactly one writer** and **exactly one reader** (single producer, single consumer — SPSC). No mutexes: coordination is done only with **atomics** and a fixed layout in shared memory.

---

## 2. High-level architecture

```
Writer process                          Reader process
     |                                        |
     |  shm_open + mmap (create)              |  shm_open + mmap (attach)
     v                                        v
     +----------------------------------------+
     |           SHARED MEMORY REGION         |
     |  total_bytes | read_pos | write_pos    |
     |  done | abort | buffer[4 MiB]         |
     +----------------------------------------+
     |                                        |
     |  run_writer_loop()                     |  wait_for_total_bytes()
     |  run_reader_loop()                     |
     v                                        v
   munmap, shm_unlink                       munmap
```

- Both processes map the **same** POSIX shared memory object (by name).
- The **layout** of that region is the `Shared` struct (see below). Writer creates and initializes it; reader attaches and reads/writes according to the protocol.

---

## 3. What we use (libraries and primitives)

### 3.1 Shared memory (OS)

- **`libc`** (Rust crate): C bindings for:
  - **`shm_open`** — create or open a POSIX shared memory object by name (e.g. `/my_ring`).
  - **`ftruncate`** — set its size to `size_of::<Shared>()`.
  - **`mmap(..., MAP_SHARED, ...)`** — map that object into the process address space so changes are visible to the other process.
  - **`munmap`** — unmap before exit.
  - **`shm_unlink`** (writer only) — delete the object after use.

We use **no other OS IPC** (no pipes, sockets, or files) for the data path. Only this one shared memory region.

### 3.2 Synchronization (no locks)

- **`std::sync::atomic`**:
  - **`AtomicU64`** for: `total_bytes`, `read_pos`, `write_pos`.
  - **`AtomicI32`** for: `done`, `abort`.
  - **Orderings**:
    - **`Ordering::Release`** on the writer when it **publishes** (e.g. after writing bytes it does `write_pos.store(..., Release)`).
    - **`Ordering::Acquire`** on the reader when it **observes** (e.g. `write_pos.load(Acquire)` before reading the buffer).

This gives a **happens-before** relationship: the reader never reads buffer bytes until after the writer has published the corresponding `write_pos`, and the writer never overwrites a region until the reader has advanced `read_pos`.

### 3.3 Data layout

- **`Shared`** in **`src/lib.rs`**:
  - **`#[repr(C)]`** so the layout is fixed and the same in both processes.
  - Fields: `total_bytes`, `read_pos`, `write_pos`, `done`, `abort`, then `buffer[4 MiB]`.
  - Writer and reader both use this same struct; the writer creates the region and inits it with **`init_shared(shm, total_bytes)`**.

### 3.4 Other

- **`std::ptr`** — `copy_nonoverlapping`, `write_bytes` for copying data in/out of the ring.
- **`sha2`** — SHA256 of the received data (after the timed section, for correctness checking).
- **`std::time::Instant`** — only around the reader’s copy loop, so throughput reflects the shared-memory transfer, not hashing or I/O.

---

## 4. Protocol (how writer and reader coordinate)

1. **Writer**
   - Creates shared memory, calls **`init_shared(shm, total_bytes)`** (sets `done=0`, `abort=0`, `read_pos=0`, `write_pos=0`, then **`total_bytes.store(..., Release)`**).
   - Runs **`run_writer_loop(shm, total_bytes)`**:
     - While `written < total_bytes`: wait until there is space (`used < BUF_SIZE`), write a chunk into `buffer`, then **`write_pos.store(..., Release)`**.
     - Then **`done.store(1, Release)`**.
     - Then spin until **`read_pos.load(Acquire) >= total_bytes`** (reader has consumed everything).
   - Unmaps and unlinks the shared object.

2. **Reader**
   - Opens the same shared memory name and mmaps it.
   - **`wait_for_total_bytes(shm)`**: spin until **`total_bytes.load(Acquire) != 0`** (so we don’t use “0” as “not yet published” in the normal path; see tests for the zero-byte case).
   - Allocates **`sink`** of size **`total_bytes`**.
   - Starts **`Instant::now()`**, then **`run_reader_loop(shm, &mut sink)`**:
     - While `read_pos < total_bytes`: get **`write_pos.load(Acquire)`**, compute `available = write_pos - read_pos`, copy `min(available, total_bytes - read_pos)` bytes from the ring into `sink`, then **`read_pos.store(..., Release)`**; if no data, check **`done`** or **`abort`** and break if set, else spin.
   - Stops the timer, computes throughput (bytes / elapsed time), then hashes **`sink[..read_pos]`** and prints (hash is **not** in the timed section).

**Abort:** If the writer sets **`abort.store(1, Release)`** instead of (or before) setting `done`, the reader sees it when no data is available and exits with partial data; we use this for robustness (e.g. partial send), not for the main throughput path.

---

## 5. Why we are sure it is correct and safe

### 5.1 Single writer, single reader (no data races)

- Only one thread in the writer process touches the shared region; only one thread in the reader process touches it. So there are **no data races** on the buffer: the only concurrent accesses are through the atomics.

### 5.2 Visibility (Release/Acquire)

- Writer **writes** buffer bytes, then does **`write_pos.store(..., Release)`**. Reader does **`write_pos.load(Acquire)`**, then **reads** the buffer. So the reader never reads a byte until the writer has “released” the corresponding write position. Similarly, reader **writes** `read_pos.store(..., Release)` and writer **loads** `read_pos` with **Acquire**, so the writer never overwrites data the reader hasn’t yet “released” as consumed. This matches the intended semantics and avoids undefined behavior from reordering.

### 5.3 Bounds and overflow safety

- **Writer**: `base` and `first` are derived from `write_pos` and `to_write`; all writes stay inside **`buffer[0..BUF_SIZE]`**.
- **Reader**: We cap **`to_read`** by **`(total_bytes - read_pos)`** so we never write past **`sink.len()`** even if a buggy or malicious writer sets **`write_pos > total_bytes`**. The test **`write_pos_exceeds_total_bytes_no_overflow`** checks this.

### 5.4 No lock-based deadlock

- We use **no mutexes or condition variables**. Only spin loops on atomics. So there is no lock ordering or classic deadlock; the only risk would be an infinite spin if the other process never updates, which is a liveness/design assumption (both processes run and follow the protocol), not a correctness bug in the synchronization primitives.

---

## 6. How we know it works: tests

All tests live in **`src/lib.rs`** in the **`#[cfg(test)] mod tests`** block. They use a **single process**: the “shared” region is a **`Box`**-allocated **`Shared`**, and we run the **writer in one thread** and the **reader in the main thread** (same memory, same layout as the real two-process case). So we are testing the **same protocol and the same core logic** that the binaries use.

| Test | What it proves |
|------|-----------------|
| **total_bytes_zero** | When `total_bytes == 0`, the reader exits immediately with 0 bytes and no infinite wait (uses `run_reader_loop_given_total` so we don’t spin in `wait_for_total_bytes`). |
| **one_byte** | Full transfer of 1 byte: writer completes, reader gets 1 byte. |
| **three_bytes** | Small transfer (3 bytes) works. |
| **full_transfer** | 10,000 bytes: writer runs to completion, reader gets all bytes, content is zeros. |
| **exactly_one_buffer** | Transfer of exactly **BUF_SIZE** (4 MiB): one full buffer, no wrap. |
| **larger_than_one_buffer** | Transfer larger than 4 MiB: the circular buffer wraps; reader still gets the correct amount. |
| **buffer_boundary_wrap** | **BUF_SIZE + 1** bytes: stresses the wrap at the boundary (first chunk to end of buffer, second chunk from start). |
| **abort_immediate** | Writer sets **abort** before sending; reader exits with 0 bytes and **aborted == true**. |
| **done_with_partial_data** | Buggy writer: **done=1** but **write_pos=50** and **total_bytes=100**. Reader stops and returns 50 bytes (does not spin forever). |
| **write_pos_exceeds_total_bytes_no_overflow** | Buggy writer: **write_pos=200**, **total_bytes=100**, **sink.len()=100**. Reader caps **to_read** so it never writes past **sink**; we assert **read_pos == 100** and **sink.len() == 100**. |

Running:

```bash
cargo test -p throughput --lib
```

should show **10 passed** and finish quickly (no hangs). This gives confidence that:

- Normal transfers (including 0, 1, small, one buffer, and multi-buffer) work.
- The circular buffer and wrap logic are correct.
- Abort and “done with partial data” are handled and do not cause infinite spins.
- The reader is safe even when **write_pos > total_bytes** (no sink overflow).

---

## 7. How to run the binaries (two processes)

1. **Start the writer first** (it creates the shared object):
   ```bash
   cargo run -p throughput --bin writer -- /my_ring 100
   ```
   This sends 100 MiB; the writer blocks until the reader has read everything.

2. **In another terminal, start the reader** (same name):
   ```bash
   cargo run -p throughput --bin reader -- /my_ring
   ```
   The reader prints throughput (e.g. MiB/s and Gb/s) and the SHA256 of the received data.

Use the **same** shared memory name in both commands (e.g. `/my_ring`). The reader must run before the writer exits, or the writer will block in the “wait for reader” loop.

---

## 8. Summary for the professor

- **What:** A two-process throughput experiment over a single POSIX shared memory region, with a fixed circular buffer and atomic positions (SPSC).
- **What we use:** `libc` for `shm_open`/`mmap`/`munmap`/`shm_unlink`; `std::sync::atomic` with Release/Acquire; a single **`#[repr(C)]`** struct for the shared layout; raw pointer copies for the buffer.
- **Why it’s safe:** Single writer and single reader (no data races); Release/Acquire for visibility; reader caps **to_read** so **sink** never overflows; no locks, so no lock-related deadlock.
- **How we know it works:** 10 unit tests in **`src/lib.rs`** run the same core logic (writer in one thread, reader in the other, same **Shared** layout) and cover normal transfers, buffer wrap, abort, and robustness (partial done, **write_pos > total_bytes**). All tests pass and complete in a fraction of a second.

This document and the test names/comments in **`src/lib.rs`** are the main references for explaining the design, safety, and validation to a professor or reviewer.
