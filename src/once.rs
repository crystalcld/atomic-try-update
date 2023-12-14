//! A wait-free alternative to `std::sync::OnceLock`, with helper methods that make it easier to
//! correctly register state at startup.
use std::{error::Error, fmt::Display, ptr::null_mut};

use num_enum::{IntoPrimitive, TryFromPrimitive};

use crate::{
    atomic_try_update,
    bits::{Align8, FlagPtr},
    Atom,
};

#[derive(IntoPrimitive, TryFromPrimitive)]
#[repr(usize)]
enum Lifecycle {
    NotSet = 0,
    Setting,
    Set,
    Dead,
}

/// Not exposed in external API.  We panic on the field `UseAfterFreeBug`, and map
/// everything else to `OnceLockFreeError` before returning it to callers.
enum OnceLockFreeInternalError {
    AlreadySet,
    AttemptToReadWhenUnset,
    AttemptToSetConcurrently,
    UseAfterFreeBug,
    UnpreparedForSet,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OnceLockFreeError {
    AlreadySet,
    AttemptToReadWhenUnset,
    AttemptToSetConcurrently,
    UnpreparedForSet,
}

impl Error for OnceLockFreeError {}

impl Display for OnceLockFreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

fn panic_on_memory_bug(err: OnceLockFreeInternalError) -> OnceLockFreeError {
    match err {
        OnceLockFreeInternalError::AlreadySet => OnceLockFreeError::AlreadySet,
        OnceLockFreeInternalError::AttemptToReadWhenUnset => {
            OnceLockFreeError::AttemptToReadWhenUnset
        }
        OnceLockFreeInternalError::AttemptToSetConcurrently => {
            OnceLockFreeError::AttemptToSetConcurrently
        }
        OnceLockFreeInternalError::UseAfterFreeBug => {
            panic!("Encountered use-after-free in OnceLockFree");
        }
        OnceLockFreeInternalError::UnpreparedForSet => OnceLockFreeError::UnpreparedForSet,
    }
}

#[derive(Default)]
struct OnceLockFreeState<T> {
    flag_ptr: FlagPtr<Align8<T>>,
}

/// A wait-free alternative to `std::sync::OnceLock`
///
/// This includes a few special purpose helper methods for various use cases.  The main advanatge
/// of these helpers is that they map unexpected states into `OnceLockFreeError` values.
///
/// The helper methods are designed to be used in pairs:
///
/// If you need to wait until a value has been registered, use `get_poll` to read it,
/// and `set` to set it.
///
/// If you need want to set a value exactly once, wait until everying is set, and then later read
/// the value you have a few options.
///
/// If you need to memoize the result, use `get_or_prepare_to_set()` to check to see if the value
/// has been set, and then use `set_prepared()` to install the value.  Do this in a way that
/// guarantees that callers will not race to set the value.  After all the sets have completed, you
/// can use `get()` or `get_or_prepare_to_set()` to read values that must be present.
///
/// If you want to guarantee that no setters succeed after the first `get()`, and don't guarantee that
/// all values are set by the time initialization completes, use `get_or_seal()`.
pub struct OnceLockFree<T> {
    inner: Atom<OnceLockFreeState<T>, u64>,
}

impl<'a, T> OnceLockFree<T> {
    /// Creates a new empty cell.
    pub fn new() -> Self {
        Default::default()
    }

    pub fn get_or_prepare_to_set(&'a self) -> Result<Option<&'a T>, OnceLockFreeError> {
        unsafe {
            Ok(
                atomic_try_update(&self.inner, |s| match s.flag_ptr.get_flag().try_into() {
                    Ok(Lifecycle::NotSet) => {
                        s.flag_ptr.set_flag(Lifecycle::Setting.into());
                        (true, Ok(None))
                    }
                    Ok(Lifecycle::Setting) => (
                        false,
                        Err(OnceLockFreeInternalError::AttemptToSetConcurrently),
                    ),
                    Ok(Lifecycle::Set) => {
                        let ptr = s.flag_ptr.get_ptr();
                        (false, Ok(if ptr.is_null() { None } else { Some(ptr) }))
                    }
                    Ok(Lifecycle::Dead) => (false, Err(OnceLockFreeInternalError::UseAfterFreeBug)),
                    Err(_) => {
                        panic!("torn read?")
                    }
                })
                .map_err(panic_on_memory_bug)?
                .map(|ptr| &(*ptr).inner),
            )
        }
    }

    /// Gets the reference to the underlying value.
    ///
    /// Unlike OnceCell and OnceLock, which return an ``Option<T>``, this returns
    /// an Error if the value has not yet been set.  There are a few other
    /// variants of get() that are appropriate for other use cases.
    pub fn get(&'a self) -> Result<&'a T, OnceLockFreeError> {
        match self.get_or_seal()? {
            Some(t) => Ok(t),
            None => Err(OnceLockFreeInternalError::AttemptToReadWhenUnset),
        }
        .map_err(panic_on_memory_bug)
    }

    /// Gets the reference to the underlying value, or None if the value has
    /// not been set yet.
    pub fn get_poll(&'a self) -> Option<&'a T> {
        unsafe {
            atomic_try_update(&self.inner, |s| match s.flag_ptr.get_flag().try_into() {
                Ok(Lifecycle::Set) => {
                    let ptr = s.flag_ptr.get_ptr();
                    (false, if ptr.is_null() { None } else { Some(ptr) })
                }
                _ => (false, None),
            })
            .map(|ptr| &(*ptr).inner)
        }
    }

    /// Gets the reference to the underyling value or "seals" self so that it
    /// can never be set to a value moving forward.
    ///
    /// Returns error if another thread concurrently prepares self, and during shutdown.
    pub fn get_or_seal(&'a self) -> Result<Option<&'a T>, OnceLockFreeError> {
        unsafe {
            Ok(
                atomic_try_update(&self.inner, |s| match s.flag_ptr.get_flag().try_into() {
                    Ok(Lifecycle::NotSet) => {
                        s.flag_ptr.set_flag(Lifecycle::Set.into());
                        s.flag_ptr.set_ptr(null_mut());
                        (true, Ok(None))
                    }
                    Ok(Lifecycle::Setting) => (
                        false,
                        Err(OnceLockFreeInternalError::AttemptToSetConcurrently),
                    ),
                    Ok(Lifecycle::Set) => {
                        let ptr = s.flag_ptr.get_ptr();
                        (false, Ok(if ptr.is_null() { None } else { Some(ptr) }))
                    }
                    Ok(Lifecycle::Dead) => (false, Err(OnceLockFreeInternalError::UseAfterFreeBug)),
                    Err(_) => {
                        panic!("torn read?")
                    }
                })
                .map_err(panic_on_memory_bug)?
                .map(|ptr| (&(*ptr).inner)),
            )
        }
    }
    /// set the value after a call to get_or_prepare_to_set returned None.  This is done in
    /// two phases so that racing sets are more likely to be noticed, and to help callers
    /// improve error messages when that happens.
    ///
    /// TODO: Add an Error state, and transition into it when racing get_or_prepare_to_set
    ///       calls occur.
    ///
    /// Returns error if already set, or if we haven't been prepared
    pub fn set_prepared(&'a self, val: T) -> Result<&'a T, OnceLockFreeError> {
        // This ensures the ptr is 8-byte aligned (or more), so that flag_ptr can steal
        // the three least significant bits
        let ptr: *mut Align8<T> = Box::into_raw(Box::new(val.into()));
        unsafe {
            atomic_try_update(&self.inner, |s| match s.flag_ptr.get_flag().try_into() {
                Ok(Lifecycle::NotSet) => (false, Err(OnceLockFreeInternalError::UnpreparedForSet)),
                Ok(Lifecycle::Setting) => {
                    s.flag_ptr.set_flag(Lifecycle::Set.into());
                    s.flag_ptr.set_ptr(ptr);
                    (true, Ok(()))
                }
                Ok(Lifecycle::Set) => (false, Err(OnceLockFreeInternalError::AlreadySet)),
                Ok(Lifecycle::Dead) => (false, Err(OnceLockFreeInternalError::UseAfterFreeBug)),
                Err(_) => {
                    panic!("torn read?")
                }
            })
            .map_err(panic_on_memory_bug)?;
            Ok(&(*ptr).inner)
        }
    }
    /// Set this to the provided value.  Wait free.
    ///
    /// Returns Error if we've been prepared, or set already, and a reference to the stored val on success.
    pub fn set(&'a self, val: T) -> Result<&'a T, OnceLockFreeError> {
        let ptr: *mut Align8<T> = Box::into_raw(Box::new(val.into()));
        unsafe {
            atomic_try_update(&self.inner, |s| match s.flag_ptr.get_flag().try_into() {
                Ok(Lifecycle::NotSet) => {
                    s.flag_ptr.set_flag(Lifecycle::Set.into());
                    s.flag_ptr.set_ptr(ptr);
                    (true, Ok(()))
                }
                Ok(Lifecycle::Setting) => (
                    false,
                    Err(OnceLockFreeInternalError::AttemptToSetConcurrently),
                ),
                Ok(Lifecycle::Set) => (false, Err(OnceLockFreeInternalError::AlreadySet)),
                Ok(Lifecycle::Dead) => (false, Err(OnceLockFreeInternalError::UseAfterFreeBug)),
                Err(_) => {
                    panic!("torn read?")
                }
            })
            .map_err(panic_on_memory_bug)?;
            Ok(&(*ptr).inner)
        }
    }
}

impl<T> Default for OnceLockFree<T> {
    fn default() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

impl<T> Drop for OnceLockFree<T> {
    fn drop(&mut self) {
        unsafe {
            match atomic_try_update(&self.inner, |s| {
                match s.flag_ptr.get_flag().try_into() {
                    Ok(Lifecycle::NotSet) => {
                        s.flag_ptr.set_flag(Lifecycle::Dead.into());
                        (true, Ok(None))
                    }
                    Ok(Lifecycle::Setting) => {
                        s.flag_ptr.set_flag(Lifecycle::Dead.into());
                        (true, Ok(None))
                    }
                    Ok(Lifecycle::Set) => {
                        s.flag_ptr.set_flag(Lifecycle::Dead.into());
                        let ptr = s.flag_ptr.get_ptr();
                        (
                            true,
                            if ptr.is_null() {
                                Ok(None)
                            } else {
                                Ok(Some(ptr))
                            },
                        )
                    }
                    Ok(Lifecycle::Dead) => {
                        // TODO: report double free (as a panic outside the atomic_try_update)
                        (false, Err(OnceLockFreeInternalError::UseAfterFreeBug))
                        // don't want to double free!
                    }
                    Err(_) => {
                        (true, Ok(None)) // CAS from torn read should fail.
                    }
                }
            })
            .map_err(panic_on_memory_bug)
            .unwrap()
            {
                None => (),
                Some(ptr) => {
                    let _drop = Box::from_raw(ptr);
                }
            };
        }
    }
}
