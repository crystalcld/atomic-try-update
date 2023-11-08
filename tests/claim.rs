use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
};

use atomic_try_update::claim::{Countable, WriteOrderingQueue};
use rand::{rngs::ThreadRng, Rng};

struct Chunk {
    sz: u64,
}

impl Countable for Chunk {
    fn get_count(&self) -> u64 {
        self.sz
    }
}

fn counting_worker(
    num_inserts: u64,
    queue: &WriteOrderingQueue<Chunk>,
    total_inserted: &std::sync::atomic::AtomicU64,
    total_dequeued: &std::sync::atomic::AtomicU64,
) {
    let mut rand = ThreadRng::default();
    let mut last_off = 0;
    for _i in 0..num_inserts {
        let count = rand.gen_range(10..10_000_000);
        total_inserted.fetch_add(count, Ordering::SeqCst);
        let chunk = Chunk { sz: count };
        let (off, claimed) = queue.push(chunk);
        assert!(off >= last_off);
        last_off += count;

        if claimed {
            let mut last_dequeue_count = total_dequeued.load(Ordering::Relaxed);
            loop {
                let (iter, claimed) = queue.consume_or_release_claim();
                if !claimed {
                    break;
                }

                for chunk in iter {
                    let new_dequeue_count =
                        total_dequeued.fetch_add(chunk.get_count(), Ordering::SeqCst);
                    // check that no other thread is in this loop.
                    assert_eq!(last_dequeue_count, new_dequeue_count);
                    last_dequeue_count += chunk.get_count();
                }
            }
        }
    }
}

const NUM_THREADS: u64 = 100;
const NUM_INSERTS: u64 = 10000;

#[test]
fn test_write_ordering_queue() {
    let queue = Arc::new(WriteOrderingQueue::<Chunk>::default());
    let total_inserted = Arc::new(AtomicU64::new(0));
    let total_dequeued = Arc::new(AtomicU64::new(0));

    let mut threads = vec![];

    for _ in 0..NUM_THREADS {
        let queue = queue.clone();
        let total_inserted = total_inserted.clone();
        let total_dequeued = total_dequeued.clone();
        threads.push(thread::spawn(move || {
            counting_worker(NUM_INSERTS, &queue, &total_inserted, &total_dequeued);
        }));
    }
    for t in threads {
        t.join().unwrap();
    }

    assert_eq!(
        queue.get_offset(),
        total_inserted.load(std::sync::atomic::Ordering::Relaxed)
    );
    assert_eq!(
        queue.get_offset(),
        total_dequeued.load(std::sync::atomic::Ordering::Relaxed)
    );
}
