#![feature(type_info, const_trait_impl, const_cmp)]

use core::marker::PhantomData;
use core::mem::type_info;

struct Point<'a> {
    x: u32,
    y: u8,
    name: &'a str,
}

enum Shape {
    Dot,
    Line(u16, u64),
}

struct Reflected<T>(PhantomData<T>);

// What bun's `MultiArrayList` asks of a type: its fields, their sizes, names and offsets,
// and whether a field is of a given type.
impl<T> Reflected<T> {
    const VARIANTS: usize = type_info::of::<T>().variants();
    const FIELDS: usize = type_info::of::<T>().fields(0);
    const SECOND_SIZE: usize = match type_info::of::<T>().field(0, 1).type_id().size() {
        Some(size) => size,
        None => 0,
    };
    const SECOND_NAME: &'static str = type_info::of::<T>().field(0, 1).name();
    const SECOND_OFFSET: usize = type_info::of::<T>().field(0, 1).offset();
    const FIRST_IS_U32: bool = type_info::of::<T>().field(0, 0).type_id() == type_info::of::<u32>();
    const THIRD_IS_STR: bool = type_info::of::<T>().field(0, 2).type_id() == type_info::of::<&str>();
}

pub fn fields() -> String {
    format!(
        "{} {} {} {} {} {} {} | {} {} {}",
        Reflected::<Point>::VARIANTS,
        Reflected::<Point>::FIELDS,
        Reflected::<Point>::SECOND_SIZE,
        Reflected::<Point>::SECOND_NAME,
        Reflected::<Point>::SECOND_OFFSET,
        Reflected::<Point>::FIRST_IS_U32,
        Reflected::<Point>::THIRD_IS_STR,
        Reflected::<Shape>::VARIANTS,
        Reflected::<Shape>::FIELDS,
        Reflected::<(u8, u32)>::SECOND_SIZE,
    )
}
