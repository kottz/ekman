use server::run;

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("server failed: {err}");
        std::process::exit(1);
    }
}
