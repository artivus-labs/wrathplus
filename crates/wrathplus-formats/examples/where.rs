//! The install WrathPlus's resolver finds, or every place it looked; exits 1 when there is none.
//! `scripts/gates.sh` asks this rather than repeating the rule.
//! `cargo run -p wrathplus-formats --example where`

fn main() -> std::process::ExitCode {
    match wrathplus_formats::wow_data() {
        Some(data) => {
            println!("{}", data.display());
            std::process::ExitCode::SUCCESS
        }
        None => {
            eprintln!("no WoW install found. Looked in, in order:");
            for c in wrathplus_formats::candidates() {
                eprintln!("  {}", c.display());
            }
            eprintln!("Set $WOW_DATA, or put a WoW folder beside the binary.");
            std::process::ExitCode::FAILURE
        }
    }
}
