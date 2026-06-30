//! Print the deterministic eval scorecard. Offline, free, reproducible.
//!
//!     cargo run -p axiom-core --example eval
//!
//! Exits non-zero if the corpus is not clean (an honesty violation, a false positive,
//! or a missed detection), so it doubles as a CI gate alongside `cargo test`.

use axiom_core::eval::{corpus, score};

fn main() {
    let card = score(&corpus());
    print!("{}", card.render());
    if !card.is_clean() {
        std::process::exit(1);
    }
}
