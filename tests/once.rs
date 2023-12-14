use std::error::Error;

use atomic_try_update::once::{OnceLockFree, OnceLockFreeError};

#[test]
fn smoke_test() -> Result<(), Box<dyn Error>> {
    let a = OnceLockFree::default();
    a.set(1u64)?;
    assert_eq!(a.get()?, &1u64);
    assert_eq!(a.set(1u64), Err(OnceLockFreeError::AlreadySet));
    assert_eq!(a.get()?, &1u64);

    let a = OnceLockFree::default();
    assert_eq!(
        a.set_prepared(1u64),
        Err(OnceLockFreeError::UnpreparedForSet)
    );

    let a = OnceLockFree::default();
    let x = a.get_or_prepare_to_set()?;
    assert_eq!(x, None);
    a.set_prepared(0u8)?;
    assert_eq!(a.get_or_prepare_to_set()?, Some(&0u8));

    let a = OnceLockFree::default();
    let x = a.get_poll();
    assert_eq!(x, None);
    a.set(1234u16)?;
    let x = a.get_poll();
    assert_eq!(x, Some(&1234_u16));

    let a = OnceLockFree::default();
    let x = a.get_or_seal()?;
    assert_eq!(x, None);
    let x = a.get_or_seal()?;
    assert_eq!(x, None);
    assert_eq!(a.set(1u8), Err(OnceLockFreeError::AlreadySet));

    let a = OnceLockFree::default();
    a.set(1u8)?;
    let x = a.get_or_seal()?;
    assert_eq!(x, Some(&1u8));
    let x = a.get_or_seal()?;
    assert_eq!(x, Some(&1u8));
    assert_eq!(a.set(1u8), Err(OnceLockFreeError::AlreadySet));

    Ok(())
}
