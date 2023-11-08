use atomic_try_update::bits::FlagU64;
use rand::{rngs::ThreadRng, Rng};

#[test]
fn test_flag_u64() {
    let mut rand = ThreadRng::default();

    for _ in 1..100_000 {
        let val = rand.gen_range(0..u64::MAX >> 1);
        let flag = rand.gen_bool(0.5);

        let mut f = FlagU64::default();
        f.set_val(val);
        assert_eq!(val, f.get_val());
        f.set_flag(flag);
        assert_eq!(flag, f.get_flag());
        assert_eq!(val, f.get_val());
        f.set_val(val);
        assert_eq!(val, f.get_val());
        assert_eq!(flag, f.get_flag());
    }
}
