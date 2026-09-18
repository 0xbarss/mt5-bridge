use mt5_bridge::Mt5Client;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read credentials from environment variables (0 / empty = attach to terminal's active account)
    let login: i64 = env::var("MT5_LOGIN")
        .unwrap_or_else(|_| "0".to_string())
        .parse()?;
    let password = env::var("MT5_PASSWORD")
        .or_else(|_| env::var("MT5_PIPE_SECRET"))
        .unwrap_or_default();
    let server = env::var("MT5_SERVER").unwrap_or_default();

    println!(
        "Connecting to MT5 bridge (login: {}, server: {})...",
        login, server
    );
    let client = Mt5Client::connect(login, &password, &server)?;

    let account = client.account_info()?;
    println!("=== Account Overview ===");
    println!("Balance:     {:.2}", account.balance);
    println!("Equity:      {:.2}", account.equity);
    println!("Profit/Loss: {:.2}", account.profit());
    println!("Margin:      {:.2}", account.margin);
    println!("Free Margin: {:.2}", account.free_margin);

    if let Some(level) = account.margin_level() {
        println!("Margin Level: {:.2}%", level);
    } else {
        println!("Margin Level: N/A (no margin in use)");
    }

    Ok(())
}
