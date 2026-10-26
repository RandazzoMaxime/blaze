use std::{io::Write, time::Duration};

#[inline(never)]
fn own() -> u64 {
    0
}

fn main() {
    let live = match ra_jit_live::Live::start() {
        Ok(live) => live,
        Err(why) => {
            println!("no live edits: {why}");
            std::process::exit(2);
        }
    };
    live.apply_as_they_come(|outcome| {
        println!("outcome: {outcome}");
        std::io::stdout().flush().unwrap();
    });
    println!("ready");
    let mut seen = u64::MAX;
    loop {
        let now = shared::answer() + own();
        if now != seen {
            println!("answer {now}");
            seen = now;
        }
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
}
