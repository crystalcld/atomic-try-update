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
    fn default() -> NonceStack<T> {
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

    #[allow(unused)]
    pub fn pop(&self) -> Option<T> {
        let node = unsafe {
            atomic_try_update(&self.head, |head: &mut NonceHead<T>| unsafe {
                head.nonce += 1;
                let ret = head.head;
                if ret.is_null() {
                    (false, ret)
                } else {
                    head.head = (*ret).next; // Use after free here.  Could segfault.
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
