//! Szybki test systemowego słownika bez uruchamiania całej aplikacji:
//! `cargo run --example spell_probe`.

fn main() {
    println!("słownik dostępny: {}", lotusmail_lib::spell::spell_available());

    // Drugie zdanie jest mieszane: poprawne angielskie słowa nie powinny
    // trafić na listę błędów, mimo że polski słownik ich nie zna.
    let texts = [
        "Witam serdecznie, przesyłam zaktualizowny grafik oraz fakture za lipiec.",
        "W załączniku deployment schedule na przyszły tydzień, please confirm asap.",
        "Please recieve the atachment and let me know tommorow.",
    ];

    for text in texts {
        let wide: Vec<u16> = text.encode_utf16().collect();
        println!("\ntekst: {text}");
        for e in lotusmail_lib::spell::spell_check(text.to_string()) {
            let from = e.start as usize;
            let to = from + e.length as usize;
            let word = String::from_utf16_lossy(&wide[from..to]);
            let tips = lotusmail_lib::spell::spell_suggest(word.clone());
            println!("  błąd: {word:20} podpowiedzi: {tips:?}");
        }
    }
}
