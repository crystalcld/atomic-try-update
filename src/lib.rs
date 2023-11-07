use std::marker::PhantomData;

// AtomicCell uses a lock-based fallback for u128 because stable rust does
// not include AtomicU128.
//
// I wonder if we could replace this with portable_atomic, which uses inline
// assembly for u128, or upstream a feature flag to crossbeam_utils to use
// portable_atomic where possible.
//
// https://docs.rs/portable-atomic/latest/portable_atomic/struct.AtomicU128.html
use crossbeam_utils::atomic::AtomicCell;

pub mod stack;

pub struct Atom<T, U> {
    union: PhantomData<T>,
    inner: AtomicCell<U>,
}

impl<T, U> Atom<T, U>
where
    U: Default + Send,
{
    pub fn new() -> Atom<T, U> {
        assert!(std::mem::size_of::<T>() <= std::mem::size_of::<U>());
        assert!(
            std::mem::size_of::<U>() <= 2
                || std::mem::size_of::<T>() > std::mem::size_of::<U>() / 2
        );
        Atom::<T, U> {
            union: PhantomData::<T> {},
            inner: Default::default(),
        }
    }
}
impl<T, U> Default for Atom<T, U>
where
    U: Default + Send,
{
    fn default() -> Self {
        assert!(std::mem::size_of::<T>() <= std::mem::size_of::<U>());
        assert!(
            std::mem::size_of::<U>() <= 4
                || std::mem::size_of::<T>() > std::mem::size_of::<U>() / 2
        );
        Self {
            union: Default::default(),
            inner: Default::default(),
        }
    }
}

// TODO: Restrict these so that ptr T is OK, but most other things are not.
// Also, it would be nice if the type of T was richer so that we could avoid
// these unsafe impls.
unsafe impl<T, U> Sync for Atom<T, U> {}
unsafe impl<T, U> Send for Atom<T, U> {}

/// This function is used to implement lock free synchronization primitives.
///
/// It is a special compare-and-swap loop that performs a hardware-level atomic
/// integer load from a piece of memory that it manages, casts the byte
/// representation of the integer that it read to a caller-provided type,
/// and then passes the result into a caller-provided lambda.
///
/// The lambda optionally updates the fields of the struct that were passed into
/// it.  If the lambda returns false, the loop terminates.  If it returns true,
/// then *atomic_try_update* attempts to compare-and-swap the new version of the
/// struct with the old version (by first casting it back to an integer).
///
/// The lambda actually returns a tuple of type `(bool, R)`.  The first field
/// is used to decide whether or not to perform a compare and swap.  The second
/// is passed back to the caller.  This allows the lambda to pass information
/// to its calling environment without sharing mutable references with other
/// invocations of itself or doing other things that the borrow checker disallows.
///
/// atomic_try_update is powerful for two reasons:
///
/// First, 128 bits is quite a lot of state and modern machines support 128-bit CAS.
/// Pointers on 64-bit machines are ~ 40-48 bits wide, so it is enough space for
/// two or three pointers, with some bits left over to encode additional state.
/// If even more bits are needed, you can play tricks such as storing offsets into
/// an array instead of memory addresses.  This allows you to pack state machines
/// and simple data structure (including stacks, registers, and even simple
/// allocators) into a single integer value, then to atomically modify your data
/// structures and state machines.
///
/// Second, simple, cursory reviews of the lambda code are enough to verify that
/// the resulting algorithm is linearizable:  i.e., that all concurrent executions
/// are equivalent to some single-threaded execution of the same code, and that
/// all observers agree on the schedule.
///
/// The rules are described in the safety section.  There is no way for
/// the rust compiler to check that your lambda obeys the provided rules, and
/// violations of them can lead to memory corruption, borrow checker invariant
/// violations, and other unsound behavior.  Therefore, we annotate the function
/// "unsafe".
///
/// In particular, if T is a Box<...>, and the lambda overwites its argument,
/// then the old value in the Box could be double-freed.
///
/// # Safety
///
/// In order to use atomic_try_update safely, make sure your code follows
/// the following three rules:
///
/// 1. The lambda must not have any side effects.  In particular, it must not
/// crash if it is provided with stale input.
///
/// 2. The results of speculative reads must not escape.  This is related to
/// the first invariant.  Since the lambda can run multiple times, you need
/// to make sure that any state it updates is updated on each iteartion of
/// the loop.  A classic mistake (from similar libraries in languages that
/// lack a borrow checker) is to capture a reference to a stack variable,
/// and then have the lambda modify it on one branch of a conditional, but
/// not the other.  Rust's borrow checker will notice most attempts to do
/// this, buy you can circumvent it by (for instance) taking a reference
/// to a mutex or an atomic value.  There's no valid reason to do such things,
/// so dont.
///
/// 3. Read set equivalence.  This is the most subtle, and most-often violated
/// of the three rules:  If the compare_and_exchange performed
/// by atomic_try_update succeeds, then all values observed by the lambda must
/// be up to date.
///
/// Early databases achieved this via locking values to prevent concurrent writes.
/// More recent systems use optimistic concurrency control to check to see if any
/// concurrent writes might have invalidated the reads a transaction performed.
///
/// Our rule allows more schedules than either of those approaches:
///
/// Concurrent writes to the transaction's read set are allowed as long as the most
/// recent of those writes installed the same bits as the ones our transaction
/// observed.
///
/// Since atomic_try_update only checks bits that live within the integer that compare
/// and swap examines, the lambda must ensure that any other values it read were
/// either not modified in race, or that such modifications installed identical state.
///
/// Read set equivalence is easily checked in many cases.  For example, it is trivially
/// true if the lambda only accesses data that was passed directly into it, and doesn't
/// read any other memory addresses.
///
/// However, some algorithms, such as stack implementations, need to derference
/// pointers, or read other data on the heap.  Such algorithms may be vulnerable
/// to the "ABA problem."  One solution is to use a nonce to get probablistic
/// correctness.  Another is to modify your operation's semantcs so that it
/// can be implemented without violating read set equivalence.
///
/// The examples include a few stack implementations that explain read set
/// equivalance in more detail.
///
/// Naive stack algorithms suffer from the following ABA issue:  It is possible
/// for pop() to read the head pointer "A", then be descheduled.  In race, the
/// head pointer and some other values are popped, then a node with the same
/// memory address as A is pushed back onto the stack.  The CAS will succeed,
/// even though the current value of head is semantically distinct from the
/// value the lambda read.
///
/// `Stack` avoids this by providing a `pop_all` method that removes everything
/// from the stack atomically.  This guarantees read set equivalence because it
/// does not read anything other than the CAS bits.
///
/// `NonceStack` uses a nonce to ensure that no pushes have been performed
/// in race with pop, which probabilistically guarantees that head was not popped
/// then pushed back on to the stack in race with a pop.
pub unsafe fn atomic_try_update<T, U, F, R>(state: &Atom<T, U>, func: F) -> R
where
    F: Fn(&mut T) -> (bool, R),
    U: Copy + Eq,
{
    let mut old = state.inner.load();
    let mut newval = old;
    loop {
        let newval_ptr: *mut U = &mut newval;
        let res;
        unsafe {
            let newval_ptr: *mut T = newval_ptr as *mut T;
            res = func(&mut *newval_ptr);
            if !res.0 {
                return res.1;
            }
        }
        match state.inner.compare_exchange(old, newval) {
            Ok(_) => return res.1,
            Err(val) => {
                old = val;
                newval = old;
            }
        }
    }
}
