mod helper_logic;

fn main() {
    helper_logic::run(std::env::args().skip(1).collect());
}
