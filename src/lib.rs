//! Primitives that make it easy to implement correct lock-free algorithms
//!
//! `atomic_try_update` is the main entry-point to this library, but (with
//! the exception of `NonceStack`) the included example code is also designed
//! to be used in production.  Each module implements a different family of
//! example algorithms.  If you simply want to use general-purpose algorithms
//! without modification, start with the public APIs of the data structures
//! in those modules.
//!
//! If you want to start implementing your own specialized lock-free logic,
//! start with this page, then read the top-level descriptions of each
//! of the modules this crate exports.
use std::{marker::PhantomData, ptr::null_mut};

// AtomicCell uses a lock-based fallback for u128 because stable rust does
// not include AtomicU128.
//
// I wonder if we could replace this with portable_atomic, which uses inline
// assembly for u128, or upstream a feature flag to crossbeam_utils to use
// portable_atomic where possible.
//
// https://docs.rs/portable-atomic/latest/portable_atomic/struct.AtomicU128.html
use crossbeam_utils::atomic::AtomicCell;

pub mod barrier;
pub mod bits;
pub mod claim;
pub mod once;
pub mod stack;

/// A wrapper that allows an instance of type T to be treated as though it is
/// an atomic integer type (in the style of a C/C++ union).  Use
/// `atomic_try_update` to access the data of type `T` stored in an `Atom<T>`.
///
/// The generic parameter `U` is the smallest unsigned integer type that is large
/// enough to hold an instance of T.  (Typically: `u64` or `u128`)
pub struct Atom<T, U> {
    union: PhantomData<T>,
    inner: AtomicCell<U>,
}

impl<T, U> Default for Atom<T, U>
where
    U: Default + Send,
{
    /// This creates a new instance of Atom, initializing the contents to
    /// all-zero bytes.
    ///
    /// TODO: Change this so that T implements Default, and then store the
    /// default instance of T in the atom?  If we do that, what happens to
    /// the uninitialized padding bytes?
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
// these.
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
/// two or three pointers with some bits left over to encode additional state.
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
/// In particular, if T is a Box<...>, and the lambda overwrites its argument,
/// then the old value in the Box could be double-freed.
///
/// # Safety
///
/// In order to use atomic_try_update safely, make sure your lambda follows
/// the following two rules:
///
/// 1. It must implement *read set equivalence*.
/// 2. It must be a pure (side-effect-free) function.
///
/// ## Rule 1: Read set equivalence
///
/// Read set equivalence is the invariant that if the current value of the
/// `Atom` matches the pre-value read by `atomic_try_update`, then all data
/// read by the lambda has the same value as it had when the lambda read it.
///
/// This concept is closely related to, but distinct from approaches used
/// in database concurrency control algorithms.
///
/// Read set equivalence trivially holds if the lambda only reads the data
/// that was passed directly into it, and doesn't follow pointers, references,
/// or otherwise observe other non-local state in the system.
///
/// It is also fine for the lambda to read data from its caller's stack
/// frame, as long as that data is immutable while atomic_try_update is
/// running.  This is most easily achieved by only capturing shared references,
/// and not capturing references to things that make use of interior mutability
/// (such as Mutex, or any of the atomic integer types).
///
/// Another common approach involves using some extra mechanism to ensure read
/// set equivalence.  For instance, some data structures include a nonce in the
/// data stored by `Atom`, and increment it on each operation.  As long as the
/// nonce does not wrap back around to exactly the same value just in time for
/// the compare and swap to run, then we know that no other operations on this
/// `Atom` have modified any state that we read in race with us.
///
/// The examples in the stack module explain read set equivalence in more detail.
///
/// ### Comparison with database concurrency control algorithms
///
/// Read set equivalence allows more schedules than typical database concurrency
/// control algorithms.  In particular, it allows write-write and read-write
/// conflicts in the case where the read set of the lambda ("transactions", in
/// their context) has been changed, but then changed back to the value the
/// transaction observed.  Two phase locking prevents such schedules by blocking
/// execution of conflicting logic, and multi-version concurrency control prevents
/// them by only examining the version number of the read set, and not its
/// contents.  (The nonce trick we mentioned above is analogous to multi-version
/// concurrency control.)
///
/// ## Rule 2: Pure functions
///
/// There are a few common ways for lambdas to fail to be pure functions,
/// ordered from most to least likely:
///
/// The value that is stored in the the `Atom` could implicitly interact
/// with global state.  For instance, it could be a `Box` and talk to the
/// allocator, or it could attempt to perform I/O.
///
/// The lambda could speculatively read a value after it has been freed.
/// Even if the lambda then discards the result without acting on it
/// (which would be safe in a garbage collected language), the act of
/// loading the freed value could read from memory that has been returned
/// to the operating system, leading to a segmentation fault.  This is
/// generally avoidable using an epoch based garbage collector, such as
/// `crossbeam_epoch`, or by maintaining a pool of reused, but never
/// freed objects for use by the data structure.
///
/// The lambda could speculatively read a value, store it on the heap
/// or in a captured stack variable, and then return true.  If the
/// compare and swap fails, the lambda runs again, and then fails to
/// overwrite the state from the failed speculative read, then it
/// violates linearizability.
///
/// Thanks to the borrow checker, it is fairly difficult to implement
/// such a bug.  In particular, if your lambda attempts to capture a
/// shared reference (`&mut`), it will fail to compile.  However, you
/// could defeat this check via internal mutability.
///
/// Finally, the lambda could crash or exhibit undefined behavior.  Other
/// versions of atomic_try_update include an "unsafe" (not in the rust
/// sense) variant that allows torn reads.  This means that the bit
/// representation of the object that is passed into your lambda could
/// be invalid, violating Rust's safety guarantees, or causing sanity
/// checks to fail, leading to panics.  Similarly, if the lambda follows
/// a pointer to something that has been marked for reuse (and therefore,
/// the compare and swap will fail), some other thread could modify that
/// object in race with the current thread's failed speculative lambda
/// invocation.
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

/// A linked list node that contains an instance of type T and a raw pointer
/// to the next entry in the node.  Since `atomic_try_update` speculatively
/// executes code, it can not handle values of `Box<T>` soundly.  Therefore,
/// this is the idiomatic way to store linked lists and stacks with
/// `atomic_try_update`.
///
/// TODO: Work out safety for this API.
#[derive(Debug)]
pub struct Node<T> {
    pub val: T,
    pub next: *mut Node<T>,
}

unsafe impl<T> Send for NodeIterator<T> {}

/// A consuming iterator over a value of type Node.
///
/// TODO: Document safety here, and (ideally) figure out how to
/// allow people to write atomic_try_update lambdas from outside
/// this package, but not write garbage to next from `safe` code.
/// That way, this API won't need an `unsafe` annotation (which
/// it is currently missing).
pub struct NodeIterator<T> {
    node: *mut Node<T>,
}

impl<T> Iterator for NodeIterator<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.node.is_null() {
            return None;
        }
        let popped: Box<Node<T>> = unsafe { Box::from_raw(self.node) };
        self.node = popped.next;
        Some(popped.val)
    }
}

impl<T> NodeIterator<T> {
    /// Takes ownership of node, in the style of Box::from_raw
    ///
    /// TODO: This could take a Box, and then we wouldn't need to add an unsafe annotation to it.
    pub fn new(node: *mut Node<T>) -> Self {
        Self { node }
    }

    pub fn rev(mut self) -> Self {
        let mut ret = Self { node: null_mut() };
        while !self.node.is_null() {
            let popped = self.node;
            unsafe {
                self.node = (*popped).next;
                (*popped).next = ret.node;
                ret.node = popped;
            }
        }
        ret
    }
}

impl<T> Drop for NodeIterator<T> {
    fn drop(&mut self) {
        for _ in self {}
    }
}
