// Minimal process-startup probe, not an Agentboard implementation.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["status"] {
        println!("agentboard: ok");
    } else {
        std::process::exit(2);
    }
}
