//! Each lint `cargo blaze clippy` has, firing, and its level set by attributes.

pub enum Never {}

#[allow(clippy::empty_enums)]
pub enum Allowed {}

#[expect(clippy::empty_enums)]
pub enum Expected {}

#[expect(clippy::empty_enums, reason = "a struct is no enum")]
pub struct NotAnEnum;

#[warn(clippy::nursery)]
pub enum Nursery {}

pub const GREETING: &'static str = "hi";
pub static NAMES: &'static [&'static str] = &["a"];

#[macro_export]
macro_rules! my_macro {
    ($($t:tt)*) => {
        0
    };
}

pub fn braces() -> Vec<i32> {
    let v = vec! {1, 2};
    let _ = my_macro!(1);
    v
}

pub fn maps(x: Option<i32>, r: Result<i32, ()>) -> i32 {
    let a = x.map(|it| it + 1).unwrap_or(0);
    let b = r.map(|it| it * 2).unwrap_or_default();
    let c = x.map(|it| it > 1).unwrap_or(false);
    let d = x.map(Some).unwrap_or(None);
    println!("{}", x.map(|it| it + 2).unwrap_or(1));
    if c { a + b } else { d.unwrap_or(0) }
}

#[deny(clippy::bool_assert_comparison)]
pub fn denied() {
    assert_eq!(braces().is_empty(), false);
}

#[cfg(test)]
mod tests {
    #[test]
    fn asserts() {
        assert_eq!(true, 1 > 0);
        assert_ne!(super::maps(None, Ok(1)) == 2, false);
        debug_assert_eq!(1 == 1, true);
    }
}
