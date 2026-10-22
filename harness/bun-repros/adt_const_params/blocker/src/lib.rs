#![feature(adt_const_params)]

use std::marker::ConstParamTy;

#[derive(PartialEq, Eq, ConstParamTy)]
enum Requirement {
    Connected,
    NotSubscriber,
}

fn check<const REQUIREMENT: Requirement>(subscribed: bool) -> bool {
    match REQUIREMENT {
        Requirement::Connected => true,
        Requirement::NotSubscriber => !subscribed,
    }
}

pub fn run() -> (bool, bool) {
    (
        check::<{ Requirement::Connected }>(false),
        check::<{ Requirement::NotSubscriber }>(true),
    )
}
