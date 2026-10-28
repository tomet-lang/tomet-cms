use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "tomet-cms",
    version,
    about = "Content management system for the Tomet ecosystem."
)]
struct Args {
    #[arg(default_value = ".")]
    path: std::path::PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    println!("tomet-cms initialized at: {:?}", args.path);
    Ok(())
}
