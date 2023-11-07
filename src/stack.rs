use super::{atomic_try_update, Atom};
use std::ptr::null_mut;

#[derive(Debug)]
pub struct Node<T> {
    pub val: T,
    pub next: *mut Node<T>,
}

unsafe impl<T> Send for NodeIterator<T> {}

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

struct Head<T> {
    head: *mut Node<T>,
}

pub struct Stack<T>
where
    T: Send,
{
    /* TODO: Need some type bounds on Atom<T>'s T to ensure this
    is safe (e.g., it fits in a U = u128; can we prevent
    people from putting a Box in here somehow?)
    */
    head: Atom<Head<T>, u64>,
}

impl<T> Default for Stack<T>
where
    T: Send,
{
    fn default() -> Stack<T> {
        Stack::<T> {
            head: Atom::<Head<T>, u64>::new(),
        }
    }
}

impl<T> Stack<T>
where
    T: Send,
{
    pub fn push(&self, val: T) {
        #[allow(unused_mut)]
        let mut node = Box::into_raw(Box::new(Node {
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
