use clap::Parser;
use taptext::Cli;

fn main() {
    let cli = Cli::parse();
    if let Err(error) = taptext::execute(cli) {
        eprintln!("エラー: {error:#}");
        std::process::exit(1);
    }
}
