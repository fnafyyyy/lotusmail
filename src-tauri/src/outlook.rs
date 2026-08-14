//! Import stopek z klasycznego Outlooka.
//!
//! Outlook przechowuje podpisy jako pliki .htm w %APPDATA%\Microsoft\Signatures
//! (HTML z Worda: kodowanie z <meta charset>, względne ścieżki obrazków do
//! katalogu „<nazwa>_pliki"). Czytamy, dekodujemy, osadzamy obrazki jako
//! data-URI i czyścimy przez sanitizer - zostaje przenośny fragment HTML.

use crate::error::{AppError, Result};
use base64::Engine;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlookSignature {
    pub name: String,
    pub html: String,
}

pub fn list() -> Result<Vec<OutlookSignature>> {
    let appdata =
        std::env::var("APPDATA").map_err(|_| AppError::Other("brak zmiennej %APPDATA%".into()))?;
    let dir = PathBuf::from(appdata).join("Microsoft").join("Signatures");
    let mut result = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(result),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_html = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("htm") || e.eq_ignore_ascii_case("html"))
            .unwrap_or(false);
        if !is_html {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        let html = decode(&bytes);
        let body = extract_body(&html);
        let inlined = inline_images(&body, path.parent().unwrap_or(Path::new(".")));
        let clean = crate::mail::sanitize_html(&inlined);
        if !has_visible_content(&clean) {
            // Plik podpisu bywa pusty (np. gdy stopka żyje w „nowym Outlooku"
            // w chmurze) - pomijamy, zamiast raportować pusty „sukces".
            continue;
        }
        result.push(OutlookSignature {
            name: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("podpis")
                .to_string(),
            html: clean,
        });
    }
    Ok(result)
}

/// Czy po odarciu z tagów zostaje jakikolwiek tekst albo obrazek?
fn has_visible_content(html: &str) -> bool {
    if html.contains("<img") {
        return true;
    }
    let mut in_tag = false;
    let text: String = html
        .chars()
        .filter(|c| {
            match c {
                '<' => in_tag = true,
                '>' => {
                    in_tag = false;
                    return false;
                }
                _ => {}
            }
            !in_tag
        })
        .collect();
    !text.replace("&nbsp;", " ").trim().is_empty()
}

/// Kodowanie z deklaracji <meta charset=…> (Word zapisuje np. windows-1250).
fn decode(bytes: &[u8]) -> String {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_lowercase();
    let encoding = head
        .find("charset=")
        .and_then(|i| {
            let label: String = head[i + 8..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            encoding_rs::Encoding::for_label(label.as_bytes())
        })
        .unwrap_or(encoding_rs::UTF_8);
    encoding.decode(bytes).0.into_owned()
}

fn extract_body(html: &str) -> String {
    let lower = html.to_lowercase();
    let start = lower
        .find("<body")
        .and_then(|i| html[i..].find('>').map(|j| i + j + 1));
    let end = lower.rfind("</body>");
    match (start, end) {
        (Some(s), Some(e)) if s < e => html[s..e].to_string(),
        _ => html.to_string(),
    }
}

/// Względne `src="…"` (obrazki z katalogu podpisu) zamienia na data-URI,
/// żeby stopka była samowystarczalna.
fn inline_images(html: &str, base: &Path) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find("src=\"") {
        let (before, after) = rest.split_at(i + 5);
        out.push_str(before);
        let Some(end) = after.find('"') else {
            out.push_str(after);
            return out;
        };
        let url = &after[..end];
        if url.starts_with("http") || url.starts_with("data:") || url.starts_with("cid:") {
            out.push_str(url);
        } else {
            let file = base.join(percent_decode(url));
            match std::fs::read(&file) {
                Ok(bytes) => {
                    let mime = match file
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_ascii_lowercase())
                        .as_deref()
                    {
                        Some("png") => "image/png",
                        Some("jpg") | Some("jpeg") => "image/jpeg",
                        Some("gif") => "image/gif",
                        _ => "application/octet-stream",
                    };
                    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                    out.push_str(&format!("data:{mime};base64,{b64}"));
                }
                Err(_) => out.push_str(url),
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn wczytuje_podpisy_uzytkownika() {
        let sigs = super::list().expect("odczyt katalogu podpisów");
        for s in &sigs {
            println!("podpis: {} ({} znaków HTML)", s.name, s.html.len());
            let sample: String = s.html.chars().take(400).collect();
            println!("początek: {sample}");
        }
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
