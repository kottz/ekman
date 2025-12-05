use ekman_server::run;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}
