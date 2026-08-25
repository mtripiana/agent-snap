mod commands;

use std::env;

fn main() {
    if let Err(err) = commands::run(env::args().skip(1).collect()) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
