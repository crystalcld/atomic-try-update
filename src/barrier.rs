//! User-friendly barriers that use `atomic_try_update` to handle startup and teardown race conditions.
use std::{error::Error, fmt::Display};

use crate::{atomic_try_update, bits::FlagU64, Atom};

pub struct ShutdownBarrierWaitResult {
    cancelled: bool,
}

pub struct ShutdownBarrierDoneResult {
    cancelled: bool,
    shutdown_leader: bool,
}

impl ShutdownBarrierWaitResult {
    /// This will return true for all waiters if at least one
    /// waiter called cancel() before shutdown.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

impl ShutdownBarrierDoneResult {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    pub fn is_leader(&self) -> bool {
        self.shutdown_leader
    }
}

/// Similar to `tokio::sync::Barrier`, but you don't need to know how
/// many waiters there will be up front.  It starts with one waiter,
/// and more can be added dynamically with `spawn()` (which will return
/// an error if invoked after shutdown).
///
/// This barrier's API handles two commonly-overlooked race conditions:
///
///  - It is OK to listen for completion before the first worker
///    starts execution.  The `wait()` will not finish until all the
///    workers that spawn complete.
///  - It is OK to start listening for completion after the last
///    worker exits.  In this case, the `wait()` will immediately
///    complete.
///
/// You can also invoke `cancel()`, which causes the wait result's
/// `is_cancelled()` method to return true for all waiters.
pub struct ShutdownBarrier {
    state: Atom<FlagU64, u64>,
    /// We send false for normal shutdown; true for cancellation
    broadcast: tokio::sync::broadcast::Sender<bool>,
}

enum WaitResult {
    StillRunning,
    Shutdown,
    Cancelled,
}

#[derive(Debug)]
enum DoneResult {
    Cancelled,
    AlreadyDone,
    ShutdownLeader,
    Running,
}

impl Default for ShutdownBarrier {
    fn default() -> Self {
        let this = Self {
            state: Default::default(),
            broadcast: tokio::sync::broadcast::channel(1).0,
        };
        unsafe {
            atomic_try_update(&this.state, |s| {
                s.set_val(1);
                (true, ())
            });
        }
        this
    }
}

#[derive(Debug)]
pub enum ShutdownBarrierError {
    AlreadyShutdown,
}

impl Error for ShutdownBarrierError {}

impl Display for ShutdownBarrierError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl ShutdownBarrier {
    /// Register another worker with the barrier.
    ///
    /// Returns Error if the barrier has already been completed.  The barrier
    ///         starts with one worker (the one that is the parent of all work),
    ///         so this will never happen if you are careful not to invoke `spawn()`
    ///         after the parent task invokes `done()`
    pub fn spawn(&self) -> Result<(), ShutdownBarrierError> {
        let already_shutdown = unsafe {
            atomic_try_update(&self.state, |s| {
                let count = s.get_val();
                if s.get_flag() || count == 0 {
                    (false, true) // already shutdown
                } else {
                    s.set_val(count + 1);
                    (true, false)
                }
            })
        };
        if already_shutdown {
            Err(ShutdownBarrierError::AlreadyShutdown)
        } else {
            Ok(())
        }
    }

    /// Inform the barrier that whatever work all the workers are performing
    /// has been cancelled.  This call causes `wait()` to return immediately
    /// with `cancelled = true`.
    pub fn cancel(&self) -> Result<(), ShutdownBarrierError> {
        let already_shutdown = unsafe {
            atomic_try_update(&self.state, |s| {
                let count = s.get_val();
                if s.get_flag() || count == 0 {
                    (false, true)
                } else {
                    s.set_flag(true);
                    (true, false)
                }
            })
        };
        if already_shutdown {
            Err(ShutdownBarrierError::AlreadyShutdown)
        } else {
            // send true for cancellation; false on success
            _ = self.broadcast.send(true);
            Ok(())
        }
    }

    /// Inform the barrier that a single worker has completed.
    ///
    /// Returns a `ShutdownBarrierDoneResult`, with `is_cancelled = true` if the
    /// pool of work protected by the barrier was cancelled, and
    /// `shutdown_leader = true` if this call to `done()` was the one that completed
    /// the pool of work.  Workers can check for `shutdown_leader = true` to
    /// perform clean up logic outside the thread of control that invokes `done()`.
    pub fn done(&self) -> Result<ShutdownBarrierDoneResult, ShutdownBarrierError> {
        let done_result = unsafe {
            atomic_try_update(&self.state, |s| {
                let count = s.get_val();
                s.set_val(count - 1);
                if s.get_flag() {
                    (true, DoneResult::Cancelled)
                } else if count == 0 {
                    (false, DoneResult::AlreadyDone)
                } else if count == 1 {
                    (true, DoneResult::ShutdownLeader)
                } else {
                    (true, DoneResult::Running)
                }
            })
        };
        match done_result {
            DoneResult::Cancelled => Ok(ShutdownBarrierDoneResult {
                cancelled: true,
                shutdown_leader: false,
            }),
            DoneResult::ShutdownLeader => {
                _ = self.broadcast.send(false);
                Ok(ShutdownBarrierDoneResult {
                    cancelled: false,
                    shutdown_leader: true,
                })
            }
            DoneResult::Running => Ok(ShutdownBarrierDoneResult {
                cancelled: false,
                shutdown_leader: false,
            }),
            DoneResult::AlreadyDone => Err(ShutdownBarrierError::AlreadyShutdown),
        }
    }

    /// Waits until the number of workers reaches zero.  This can be called at any time
    /// and can be called multiple times.
    pub async fn wait(&self) -> Result<ShutdownBarrierWaitResult, ShutdownBarrierError> {
        // We have to subscribe before we check state.  Otherwise, some some thread
        // could send the shutdown message after our subscription begins!
        let mut rx = self.broadcast.subscribe();
        let wait_result = unsafe {
            atomic_try_update(&self.state, |s| {
                let count = s.get_val();
                if s.get_flag() {
                    (false, WaitResult::Cancelled)
                } else if count == 0 {
                    (true, WaitResult::Shutdown)
                } else {
                    (false, WaitResult::StillRunning)
                }
            })
        };
        match wait_result {
            WaitResult::StillRunning => {
                let cancelled = rx
                    .recv()
                    .await
                    .map_err(|_| ShutdownBarrierError::AlreadyShutdown)?;
                Ok(ShutdownBarrierWaitResult { cancelled })
            }
            WaitResult::Shutdown => Ok(ShutdownBarrierWaitResult { cancelled: false }),
            WaitResult::Cancelled => Ok(ShutdownBarrierWaitResult { cancelled: true }),
        }
    }
    /// Returns a new shutdown barrier with a single worker.  The caller
    /// should spawn() all the work that needs to be done, then invoke
    /// done().  This makes sure the worker count doesn't spuriously
    /// reach zero while work is being spawned.
    pub fn new() -> Self {
        Default::default()
    }
}
