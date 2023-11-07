# atomic-try-update

This library makes it easy to correctly implement your own lock free data structures.  In addition to the base primitives, we provide a few example data structures that you can use directly, or that you can use as a base for your own application-specific algorithms.

# Overview

Typical use cases for `atomic_try_update` include implementing state machines, building simple resource allocators, initializing systems in a deterministic way using "fake monotonicity", accumulating state in stacks, and using the claim pattern to allow concurrent code to enqueue and then process them sequentially.

Unlike most lock free libraries, we make it easy to compose the above in a way that preserves linearizable semantics.  For instance, you implement a lock free state machine that tallies votes as part of a two phase commit protocol, and then combine it with a stack.  The resulting code would add information about each response to the stack and then process the result of the tally exactly once without resorting to additional synchronization such as mutexes or carefully ordered writes.

By "linearizable", we mean that any schedule of execution of the algorithms built using `atomic_try_update` is equivalent to some single-threaded schedule, and that other code running in the system will agree on the order of execution of the requests.  This is approximately equivalent to "strict serializability" from the database transaction literature.  `atomic_try_update` provides semantics somewhere between those of a transaction processing system and those of a CPU register.  We chose the term linearizable because it is more frequently used when discussing register semantics, and `atomic_try_update` is generally limited to double word (usually 128-bit) updates.

From a performance perspective, `atomic_try_update` works best when you can have many independent instances that each have low contention.  For instance, using a single `atomic_try_update` instance to coordinate all reads in a system would likely create a concurrency bottleneck.  Having one for each client connection probably would not.  This means that you should stick to other, more specialized algorithms for things like top-level event queues and other high-contention singleton data structures in your system.

# Acknowledgements
This library distills algorithmic work done by many people over multiple decades.  However, we have not been able to find any written documentation of this approach to lock free algorithm design.  If you are aware of early research or systems in this space, please reach out so we can update this section.

This work was possible because multiple generations of our colleagues trained their peers to apply these these algorithms.  Thank you.

The attendees of Dagstuhl Seminar 21442 "Ensuring the Reliability and Robustness of Database Management Systems" provided helpful feedback on a pre-release version of this library.  The deterministic sample code is inspired by the CALM principle, and similar primitives were independently invented by Joe Hellerstein's research group.  They invented the term "fake monotonicity" to characterize concurrent algorithms that either produce a deterministic result or fail with a runtime error.