#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = ruby_templates::cli::legacy(&args) {
        std::process::exit(code);
    }
    ruby_templates::serve().await;
}
