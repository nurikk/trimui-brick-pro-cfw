use launch_contract::fixture_journey;

fn main() {
    match fixture_journey() {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("launch-contract fixture journey failed: {error}");
            std::process::exit(1);
        }
    }
}
