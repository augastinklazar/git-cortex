use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "git-cortex", version, about = "Git-native epistemic context protocol")]
struct Cli {
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    let _args = Cli::parse();
    println!("git-cortex v0.1.0");
}
