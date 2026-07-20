1. Naming (codify existing practice)
- Types: UpperCamelCase
- Functions, methods, variables, modules: snake_case
- Constants and statics: UPPER_SNAKE_CASE (≤ 64 chars; longer → SCREAMING_CAMEL with comment)
- File names: snake_case.rs with no exceptions
- One public type per file when feasible

2. File structure (codify existing practice)
- First 3–5 lines: // design header explaining purpose + key references
- Imports in order: core/alloc → crate::* → external (none currently)
- extern "C" platform ABI block at file bottom, grouped
- const/static before types before impl before trait before tests

3. unsafe policy (tighten current practice)
- Every unsafe block must have a // SAFETY: comment naming each invariant the caller must uphold
- unsafe impl Send/Sync must justify why the type is safe to share — not just "lock protects it"
- Raw pointer deref requires: validity, alignment, provenance (all three named)
- unsafe fn signatures must document invariants in their /// doc, not just // SAFETY:
- unsafe_op_in_unsafe_fn = "deny" — no implicit unsafe inside unsafe fns
- #![forbid(unsafe_op_in_unsafe_fn)] at crate root

4. Pointer/ownership policy (currently ad hoc)
- Box<T>: sole owner, heap, dropped with the box
- &'a T / &'a mut T: prefer over pointers whenever the lifetime is statically known
- *const T / *mut T: only when (a) storing in a !Send intrusive node, (b) writing to a memory-mapped region, or (c) FFI
- NonNull<T> preferred over *mut T for non-null invariants
- Never *const T where &T suffices
- Intrusive pointers (e.g. run_queue_next) must have their owning lock named in the field's doc

5. Lock selection (currently ambiguous — the big one)
Use in this order of preference:
Contention pattern	Use
Single-producer single-consumer	Lock-free SPSC ring (ipc/ring_buffer.rs)Build·GLM-5.2OpenRouter
Single-writer many-reader	Atomic* + RCU-style read
/home/lex-studio/research/sydonOS/zydonCross-core counter/state	Atomic* with explicit ordering
Short critical section (< 1µs), IRQ-safe	Spinlock
Task-level mutual exclusion, can await	AsyncMutex
Task-level signaling, can await	WaitQueue + Oneshot/Broadcast
Cross-process synchronization	Futex
Hard rules:
- Never await while holding a Spinlock. Lint: #[deny(clippy::await_holding_lock)].
- Never hold a Spinlock across a function call that may block or call into platform code.
- Spinlock critical sections must be bounded by a written loop bound in the // SAFETY: comment.
- Interrupt handlers may only use Spinlock or atomics, never async primitives.
- Spinlock acquire + release must be in the same function (no SpinlockGuard returned across await).

6. Lock ordering (currently one-off in handle.rs:196)
- Document the global lock order in CONCURRENCY.md
- Lower-pointer-address-first is the existing rule for HandleTable::transfer — codify for all multi-lock paths
- Acquire locks via a lock_in_order(a, b) helper that asserts order in debug builds
- No lock acquired inside a function whose caller might already hold it without a documented exception

7. Memory ordering (currently inconsistent)
- Default to Ordering::Relaxed for stats/counters
- Acquire/Release pairs for publishing data through a shared atomic
- AcqRel only for read-modify-write on shared state
- SeqCst requires explicit justification in the comment — never the default
- Document the ordering choice at every atomic op that isn't Relaxed

8. Error handling 
- Public API returns Result<T, E> where failure is recoverable
- Option<T> where absence is normal (lookup miss, empty queue)
- panic!/expect only for invariant violations the kernel cannot recover from — and only in debug_assert!-style fashion so release builds degrade
- unreachable! requires a comment proving the case is impossible
- todo!() / unimplemented!() are banned; use panic!("not yet: <reason>") if absolutely necessary and track in TODO.md
- Error enums must carry context (which handle, which gen, which core) — no bare 4-variant enums

9. Recursion 
- No recursion inside a Spinlock critical section. The kernel stack is 8KB (KERNEL_STACK_SIZE); unbounded recursion = stack overflow = double fault.
- buddy::alloc_inner and free_and_coalesce must be rewritten iteratively
- run_queue::dequeue retry must be a loop, not self-recursion
- Any new recursive function must have a documented depth bound ≤ 16

10. Hot-path policy 
- No extern "C" platform calls inside a spinlock-protected allocator hot path — cache direct_map_base in a static AtomicUsize initialized once at boot
- No Vec allocation in IRQ context
- No format-string allocation in panic paths — use panic_str-style bare messages
- #[inline] on functions ≤ 4 lines that are called in the scheduler poll loop
- current_core_id() must read from a per-core static (e.g. GS-relative on x86), not an extern "C" call

11. Banned features
- unwrap() in non-test code (use expect("<reason>") minimum, prefer ?)
- todo!(), unimplemented!(), unreachable!() without proof comment
- Box::leak outside boot code
- mem::forget outside typed drop-replacement sites
- unsafe impl Send/Sync on a type containing Cell/RefCell
- std::* (already no_std — keep it)
- Recursion inside locks (see 9)
- await inside a Spinlock guard (see 5)

12. Documentation policy 
- Every pub item gets /// doc
- Every file gets a // design header citing references where applicable
- Every unsafe block gets // SAFETY: comment naming each invariant
- Every atomic op not using Relaxed gets a // Ordering: comment justifying the choice
- Every lock acquired must name the lock-order position in a comment when 2+ locks are taken in the same scope
- CONCURRENCY.md: list all cross-core shared state and the lock protecting each
- INVARIANTS.md: list every debug_assert! and the invariant it guards
