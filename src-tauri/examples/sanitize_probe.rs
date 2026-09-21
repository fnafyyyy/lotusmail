//! Sonda: co sanityzacja robi z danym plikiem HTML.
//! `cargo run --example sanitize_probe -- plik.html [wzorzec] [plik_wyjściowy]`
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let html = std::fs::read_to_string(&args[1]).expect("nie mogę odczytać pliku");
    let clean = lotusmail_lib::mail::sanitize_html(&html);
    let pat = args.get(2).map(String::as_str).unwrap_or("background");
    println!("wejście: {} B, wyjście: {} B", html.len(), clean.len());
    for (i, _) in clean.match_indices(pat) {
        let start = i.saturating_sub(120);
        let end = (i + 300).min(clean.len());
        let s = &clean[clean.floor_char_boundary(start)..clean.ceil_char_boundary(end)];
        println!("--- {s}");
    }
    if let Some(out) = args.get(3) {
        std::fs::write(out, &clean).expect("nie mogę zapisać wyniku");
    }
}
