//! Examples of the claim mutual exclusion pattern, including a
//! claim_queue, which allows multiple workers to enqueue work
//! and ensures that exactly one worker running if there is work
//! to be done.
//!
//! TODO: The example claim queue is strange, since it combines
//! a counter with the claim queue logic.  This is a decent example
//! of composing semi-related algorithms with atomic_try_update,
//! but it is unclear whether the example is general-purpose enough
//! to be included here.

use std::ptr::null_mut;

use super::{atomic_try_update, bits::FlagU64, Atom, Node, NodeIterator};
/// A special purpose trait for Count
pub trait Countable {
    fn get_count(&self) -> u64;
}

struct CountingClaimHead<T: Countable> {
    next: *mut Node<T>,
    /// Number of bytes inserted into this queue so far (according to Countable::get_count).
    /// The flag is the claim bit. The invariant is that if the queue is non-empty, then
    /// it is claimed by something (so the claim bit is set).  Strictly speaking, we could
    /// store the claim bit implicitly for this use case, but this is a common pattern, and
    /// we leave it explicit so this data structure can be used as example code.
    count_and_claim: FlagU64,
}

pub struct WriteOrderingQueue<T>
where
    T: Send + Countable,
{
    head: Atom<CountingClaimHead<T>, u128>,
}

impl<T> Default for WriteOrderingQueue<T>
where
    T: Send + Countable,
{
    fn default() -> WriteOrderingQueue<T> {
        WriteOrderingQueue::<T> {
            head: Atom::default(),
        }
    }
}

/// This is a multi-producer "claim" queue.
impl<T> WriteOrderingQueue<T>
where
    T: Send + Countable,
{
    /// This returns the offset of the write, and true iff we have the claim.
    /// If we have the claim, we are responsible for calling consume_or_release_claim
    /// until we manage to release it.
    pub fn push(&self, val: T) -> (u64, bool) {
        let sz = val.get_count();
        #[allow(unused_mut)]
        let mut node = Box::into_raw(Box::new(Node {
            val,
            next: std::ptr::null_mut(),
        }));

        unsafe {
            atomic_try_update(&self.head, |head: &mut CountingClaimHead<T>| {
                (*node).next = head.next;
                head.next = node;
                let old_count = head.count_and_claim.get_val();
                let have_claim = !head.count_and_claim.get_flag();
                // TODO: need to check for overflow without panic
                head.count_and_claim.set_val(old_count + sz);
                head.count_and_claim.set_flag(true); // either it was already set to true, or we need to set it to true!
                (true, (old_count, have_claim))
            })
            // Can safely panic on overflow here.
        }
    }
    /// This removes everything from the queue.  If queue is already empty, it releases the claim and returns false
    pub fn consume_or_release_claim(&self) -> (NodeIterator<T>, bool) {
        let (node, had_claim, claimed) = unsafe {
            atomic_try_update(&self.head, |head| {
                let ret = head.next;
                let had_claim = head.count_and_claim.get_flag();
                head.next = null_mut();
                if ret.is_null() {
                    head.count_and_claim.set_flag(false);
                    (true, (ret, had_claim, false)) // no longer have claim
                } else {
                    (true, (ret, had_claim, true))
                }
            })
        };
        assert!(
            had_claim,
            "cannot call consume_or_release_claim unless you have the claim!"
        );
        (NodeIterator::new(node).rev(), claimed)
    }

    pub fn get_offset(&self) -> u64 {
        unsafe { atomic_try_update(&self.head, |head| (false, head.count_and_claim.get_val())) }
    }
}
