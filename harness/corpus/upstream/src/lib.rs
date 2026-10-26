//! A library rustc compiles, whose generic code `cases` instantiates: a `TypeId` of a type
//! it names has to be rustc's.

use std::any::Any;

pub struct Wrap<T>(pub T);

pub enum Shape<T> {
    Dot,
    Line(T),
    Pair(T, T),
}

pub struct Meters(pub f32);

/// A stream mapped through `Ok` boxed by rustc's code, as tonic's client builds its requests.
pub fn boxed_ok() -> (impl Iterator<Item = Result<u8, ()>>, Box<dyn Any>) {
    (std::iter::once(1).map(Ok), Box::new(std::iter::once(2).map(Ok::<u8, ()>)))
}
