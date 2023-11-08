//! # Lightweight lock-free stack implementations
//! It is surprisingly difficult to correctly use and implement a lock-free
//! stack that provides the standard push/pop interface.
//!
//! In particular, pop has to traverse a pointer to obtain the next node in
//! the stack.  It is possible that the target of the pointer changes in
//! race (due to multiple concurrent stack pops), or even that the old head
//! was popped and deallocated and then a new entry with the same address
//! was allocated and pushed back on as the head of the stack.  At this point
//! a compare and swap of the old head with the new one would incorrectly
//! succeed, leading to an instance of the "ABA problem."
//!
//! Naive stack algorithms suffer from the following ABA issue:  It is possible
//! for pop() to read the head pointer "A", then be descheduled.  In race, the
//! head pointer and some other values are popped, then a node with the same
//! memory address as A is pushed back onto the stack.  The CAS will succeed,
//! even though the current value of head is semantically distinct from the
//! value the lambda read.
//!
//! `Stack` avoids this by providing a `pop_all` method that removes everything
//! from the stack atomically.  This guarantees read set equivalence because it
//! does not read anything other than the CAS bits.
//!
//! `NonceStack` uses a nonce to ensure that no pushes have been performed
//! in race with pop, which probabilistically guarantees that head was not popped
//! then pushed back on to the stack in race with a pop.

//!
use super::{atomic_try_update, Atom, Node, NodeIterator};
use std::ptr::null_mut;

struct Head<T> {
    head: *mut Node<T>,
}

pub struct Stack<T>
where
    T: Send,
{
    head: Atom<Head<T>, u64>,
}

impl<T> Default for Stack<T>
where
    T: Send,
{
    fn default() -> Self {
        Self {
            head: Default::default(),
        }
    }
}

impl<T> Stack<T>
where
    T: Send,
{
    pub fn push(&self, val: T) {
        let node = Box::into_raw(Box::new(Node {
            val,
            next: std::ptr::null_mut(),
        }));

        unsafe {
            atomic_try_update(&self.head, |head: &mut Head<T>| {
                (*node).next = head.head;
                head.head = node;
                (true, ())
            });
        }
    }
    pub fn pop_all(&self) -> NodeIterator<T> {
        NodeIterator {
            node: unsafe {
                atomic_try_update(&self.head, |head: &mut Head<T>| {
                    let ret = head.head;
                    head.head = null_mut();
                    (true, ret)
                })
            },
        }
    }
}

impl<T> Drop for Stack<T>
where
    T: Send,
{
    fn drop(&mut self) {
        self.pop_all();
    }
}

pub struct NonceHead<T> {
    head: *mut Node<T>,
    nonce: u64,
}

impl<T> Default for NonceStack<T>
where
    T: Send,
{
    #[allow(unreachable_code)]
    fn default() -> NonceStack<T> {
        todo!("This example code contains a use after free.");
        NonceStack::<T> {
            head: Default::default(),
        }
    }
}

pub struct NonceStack<T>
where
    T: Send,
{
    head: Atom<NonceHead<T>, u128>,
}

impl<T> NonceStack<T>
where
    T: Send,
{
    #[allow(unused)]
    pub fn push(&self, val: T) {
        let node = Box::into_raw(Box::new(Node {
            val,
            next: std::ptr::null_mut(),
        }));

        unsafe {
            atomic_try_update(&self.head, |head| {
                (*node).next = head.head;
                head.nonce += 1;
                head.head = node;
                (true, ())
            })
        }
    }

    /// Almost-correct example of using a nonce to implement pop().
    ///
    /// This method contains a use-after-free on the line `head.head=(*ret).next`.
    ///
    /// This would be safe in a garbage collected system, or in an embedded
    /// system without memory protection, since head.head will be discarded
    /// if the stack has been changed, and *ret can only be freed after it is
    /// popped from the stack.  Since *ret may have been freed, it may have
    /// been returned to the operating system, leading to a segmentation fault
    /// when accessed here.
    ///
    /// If you need a stack similar to NonceStack, consider using an epoch
    /// collector such as crossbeam_epoch, or by maintaining a pool of
    /// reusable objects.  For instance, you could use a second stack to
    /// store the pool, and return objects to it after they are popped from
    /// this stack.  If the pool stack is empty, then allocate a new object.
    ///
    /// Once you are sure that no thread will access either stack, you can
    /// safely empty both stacks and free the objects they contain.
    ///
    /// Stacks with nonces are also sometimes used to implement slot allocators.
    /// A slot allocator is initialized at startup with a finite number of
    /// tokens (such as file handles, or some other finite resource).  When
    /// a thread needs a resource, it pops from the stack.  If the stack is
    /// empty, then the thread goes async.  Atomically checking that the stack
    /// is empty and registering oneself for future wakeup is left as an exercise
    /// to the reader, as it is exactly the sort of thing atomic_try_update excels
    /// at.
    ///
    /// TODO: Implement a double-stack structure and/or slot such as the ones above,
    /// so we have correct examples of the NonceStack pattern.

    #[allow(unused)]
    pub fn pop(&self) -> Option<T> {
        let node = unsafe {
            atomic_try_update(&self.head, |head: &mut NonceHead<T>| unsafe {
                head.nonce += 1;
                let ret = head.head;
                if ret.is_null() {
                    (false, ret)
                } else {
                    head.head = (*ret).next;
                    (true, ret)
                }
            })
        };

        if !node.is_null() {
            Some(unsafe { *Box::from_raw(node) }.val)
        } else {
            None
        }
    }
}

impl<T> Drop for NonceStack<T>
where
    T: Send,
{
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}
