use mt5_bridge::Mt5Client;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read credentials from environment variables or use test defaults
    let login: i64 = env::var("MT5_LOGIN")
        .unwrap_or_else(|_| "5056168447".to_string())
        .parse()?;
    let password = env::var("MT5_PASSWORD").unwrap_or_else(|_| "demo_password".to_string());
    let server = env::var("MT5_SERVER").unwrap_or_else(|_| "MetaQuotes-Demo".to_string());

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
