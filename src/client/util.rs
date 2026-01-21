use std::fmt::Display;
use std::process;


pub fn exit(msg: &str) -> ! {
    eprintln!("{}", msg);
    process::exit(1);
}

pub fn require<T, E>(result: Result<T, E>, msg: &str) -> T
where E: Display
{
    match result {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}: {}", msg, e);
            process::exit(1);
        }
    }
}
