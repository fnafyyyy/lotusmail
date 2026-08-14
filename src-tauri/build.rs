fn main() {
    // Ikona aplikacji (pasek zadań, plik .exe) jest wkompilowywana tutaj.
    // Po zmianie plików w `icons/` trzeba powtórzyć ten skrypt — bez tego
    // Cargo uznaje build za aktualny i w binarce zostaje stara ikona.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
