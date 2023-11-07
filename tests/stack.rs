use std::{
    error::Error,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

use atomic_try_update::stack::*;

fn worker(num_inserts: u64, n: u64, stack: &Stack<u64>, total: &std::sync::atomic::AtomicU64) {
    let mut count = 0;
    for i in 0..num_inserts {
        stack.push(n * num_inserts + i);
        if i % 17 == 0 {
            let node_it = stack.pop_all();
            for _next in node_it {
                count += 1;
            }
        }
    }
    let node_it = stack.pop_all();

    for _next in node_it {
        count += 1;
    }
    total.fetch_add(count, std::sync::atomic::Ordering::SeqCst);
}

const NUM_THREADS: u64 = 100;
const NUM_INSERTS: u64 = 10000;

#[test]
fn test_stack() {
    use std::thread;
    let stack: Stack<u64> = Default::default();
    // check empty stack case.
    let mut node_it = stack.pop_all();
    assert_eq!(node_it.next(), None);
    let total = std::sync::atomic::AtomicU64::new(0);
    let start = std::time::Instant::now();
    thread::scope(|s| {
        for n in 0..NUM_THREADS {
            // Refer to stack by reference here.  This avoids using Arc<>, but now
            // the borrow checker needs to confirm that stack outlives our threads.
            let stack = &stack;
            let total = &total;
            s.spawn(move || {
                worker(NUM_INSERTS, n, stack, total);
            });
        }
    });
    assert_eq!(
        total.load(std::sync::atomic::Ordering::SeqCst),
        NUM_THREADS * NUM_INSERTS
    );
    let duration = start.elapsed().as_micros();
    println!("time elapsed (usec) {duration}");
}

#[test]
fn test_nonce_stack() {
    use std::thread;
    let stack: NonceStack<u64> = Default::default();
    assert!(stack.pop().is_none());

    let total = 250_000u64;
    let pushed = std::sync::atomic::AtomicU64::new(0);
    let popped = std::sync::atomic::AtomicU64::new(0);

    thread::scope(|s| {
        for _n in 0..NUM_THREADS {
            let stack = &stack;
            let pushed = &pushed;
            let popped = &popped;
            let total = &total;
            s.spawn(move || loop {
                let mut done = true;
                let val = pushed.fetch_add(1, Ordering::Relaxed);
                if val < *total {
                    stack.push(val);
                    done = false;
                }
                if let Some(_popped) = stack.pop() {
                    popped.fetch_add(1, Ordering::Relaxed);
                    done = false;
                }
                if done {
                    break;
                }
            });
        }
    });
    assert!(stack.pop().is_none());
    assert_eq!(popped.load(Ordering::Relaxed), total);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tokio_stack() -> Result<(), Box<dyn Error>> {
    let stack: Stack<u64> = Default::default();

    // check empty stack case.
    let mut node_it = stack.pop_all();
    assert_eq!(node_it.next(), None);
    let total = AtomicU64::new(0);
    let start = Instant::now();
    let mut workers = vec![];

    let stack = Arc::new(stack);
    let total = Arc::new(total);
    for n in 0..NUM_THREADS {
        // We move the Arcs into the lambda at spawn, but worker
        // takes them by reference (so we know their ref count
        // bumps don't interfere with the test)
        let stack = stack.clone();
        let total = total.clone();
        workers.push(tokio::spawn(async move {
            worker(NUM_INSERTS, n, &stack, &total);
        }));
    }
    for w in workers {
        w.await?;
    }
    assert_eq!(
        total.load(std::sync::atomic::Ordering::SeqCst),
        NUM_THREADS * NUM_INSERTS
    );
    let duration = start.elapsed().as_micros();
    println!("time elapsed (usec) {duration}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_node_iterator_reverse() {
    let stack: Stack<u64> = Default::default();

    for i in 1..100 {
        stack.push(i);
    }

    let mut iter = stack.pop_all().rev();
    for i in 1..100 {
        assert_eq!(iter.next().unwrap(), i);
    }
    assert_eq!(iter.next(), None);
}
