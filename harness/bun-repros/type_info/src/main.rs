#![feature(type_info)]

use core::marker::PhantomData;
use core::mem::type_info;

struct Point {
    x: u32,
    y: u8,
}

struct Reflected<T>(PhantomData<T>);

impl<T> Reflected<T> {
    const FIELDS: usize = type_info::of::<T>().fields(0);
}

fn fields() -> usize {
    Reflected::<Point>::FIELDS
}

fn main() {
    let point = Point { x: 1, y: 2 };
    let _ = (point.x, point.y);
    println!("{}", fields());
}
