//! Bit packing and pointer alignment utilities that make it easier to fit
//! additional state into an `Atom<T>`

use std::marker::PhantomData;

/// A packed pointer type that steals some bits to
/// make room for a 3-bit flag
///
/// T must be aligned to 8 bytes:
/// ```
/// # #[repr(align(8))]
/// struct MyStruct { }
/// ```
///
/// If you don't control the alignment requirements of `T`, consider using the
/// wrapper `Align8<T>` in this module instead:
/// ```
/// # use atomic_try_update::bits::{Align8, FlagPtr};
/// struct MaybeUnaligned { b: bool }
/// let ptr : FlagPtr<Align8<MaybeUnaligned>> = Default::default();
/// ```
pub struct FlagPtr<T> {
    val: usize,
    _phantom: PhantomData<T>,
}
impl<T> Default for FlagPtr<T> {
    fn default() -> Self {
        Self {
            val: 0,
            _phantom: Default::default(),
        }
    }
}
impl<T> FlagPtr<T> {
    // Assuming 8 byte alignment.
    const MASK: usize = 0b111;
    pub fn get_ptr(&self) -> *mut T {
        (self.val & !Self::MASK) as *mut T
    }
    /// This function panics if ptr is not 8 byte aligned.
    pub fn set_ptr(&mut self, ptr: *mut T) {
        let ptr = ptr as usize;
        assert_eq!(ptr & Self::MASK, 0);
        self.val = (ptr & !Self::MASK) | (self.val & Self::MASK);
    }
    pub fn get_flag(&self) -> usize {
        self.val & Self::MASK
    }
    /// This function panics if flag is greater than seven (0b111).
    pub fn set_flag(&mut self, flag: usize) {
        assert_eq!(flag & !Self::MASK, 0);
        self.val = (self.val & !Self::MASK) | (flag & Self::MASK);
    }
}

/// Bottom bit is the flag; you get 63 bits for val.
#[derive(Default)]
pub struct FlagU64 {
    val: u64,
}

impl FlagU64 {
    pub fn get_val(&self) -> u64 {
        self.val >> 1
    }

    pub fn get_flag(&self) -> bool {
        (self.val & 0x1) == 1
    }

    pub fn set_val(&mut self, val: u64) {
        self.val = (self.val & 0x1) | (val << 1); // TODO: Check for overflow!
    }
    pub fn set_flag(&mut self, flag: bool) {
        self.val = (self.val & !0x1) | u64::from(flag)
    }
}

/// Bottom bit is the flag; you get 31 bits for val.
pub struct FlagU32 {
    val: u32,
}

impl FlagU32 {
    pub fn get_val(&self) -> u32 {
        self.val >> 1
    }

    pub fn get_flag(&self) -> bool {
        (self.val & 0x1) == 1
    }

    pub fn set_val(&mut self, val: u32) {
        self.val = (self.val & 0x1) | (val << 1); // TODO: Check for overflow!
    }
    pub fn set_flag(&mut self, flag: bool) {
        self.val = (self.val & !0x1) | u32::from(flag)
    }
}

/// A wrapper around an instance of T that is aligned on an eight
/// byte boundary.  This allows FlagPtr to steal the bottom three
/// bits of pointers to instances of T without worrying about T's
/// alignment requirements.
///
/// TODO: Use Deref or something to make this transparently act
/// like a T?
#[repr(align(8))]
pub struct Align8<T> {
    pub inner: T,
}

impl<T> From<T> for Align8<T> {
    fn from(inner: T) -> Self {
        Align8 { inner }
    }
}
