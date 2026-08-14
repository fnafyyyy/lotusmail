//! Sprawdzanie pisowni oparte o systemowy silnik Windows (ISpellChecker).
//!
//! Świadomie nie dokładamy własnych słowników: Windows ma je już zainstalowane
//! razem z pakietami językowymi, zna odmianę polską i pozwala trwale dopisać
//! słowo do słownika użytkownika. Działa w całości offline.
//!
//! Sprawdzamy dwujęzycznie - polskim i angielskim naraz. Słowo jest błędem
//! dopiero wtedy, gdy nie zna go żaden ze słowników, więc angielskie wtręty
//! w polskim mailu (i odwrotnie) nie są podkreślane.
//!
//! Uwaga na indeksy: WinAPI liczy pozycje w jednostkach UTF-16 - dokładnie tak
//! samo jak łańcuchy w JavaScripcie, więc granice błędów trafiają do interfejsu
//! bez przeliczania.

use crate::error::{AppError, Result};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpellError {
    /// Pozycja początku słowa liczona w jednostkach UTF-16.
    pub start: u32,
    pub length: u32,
    /// Podpowiedź podana przez system (autokorekta) - zwykle pusta,
    /// pełną listę pobiera się osobno przez `spell_suggest`.
    pub replacement: Option<String>,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Globalization::{
        ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory, CORRECTIVE_ACTION_GET_SUGGESTIONS,
        CORRECTIVE_ACTION_REPLACE,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoIncrementMTAUsage, CLSCTX_INPROC_SERVER};

    /// Języki, w których sprawdzamy pisownię. Z każdej grupy bierzemy pierwszy
    /// wariant obecny w systemie, a potem używamy wszystkich naraz.
    const LANGUAGES: &[&[&str]] = &[&["pl-PL", "pl"], &["en-US", "en-GB", "en"]];

    thread_local! {
        /// Utworzenie obiektów COM kosztuje, a sprawdzamy przy każdej pauzie
        /// w pisaniu - trzymamy je więc per wątek puli Tauriego.
        static CHECKERS: RefCell<Option<Vec<ISpellChecker>>> = const { RefCell::new(None) };
    }

    /// Utrzymuje przy życiu apartament wielowątkowy. Bez tego wątki puli,
    /// które nigdy nie wywołały CoInitializeEx, nie mogą tworzyć obiektów COM.
    fn ensure_com() {
        static MTA: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        MTA.get_or_init(|| unsafe {
            CoIncrementMTAUsage().ok();
        });
    }

    fn create() -> Vec<ISpellChecker> {
        ensure_com();
        let mut out = Vec::new();
        unsafe {
            let factory: ISpellCheckerFactory =
                match CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER) {
                    Ok(f) => f,
                    Err(_) => return out,
                };
            for group in LANGUAGES {
                for lang in *group {
                    let lang = HSTRING::from(*lang);
                    let supported = factory.IsSupported(PCWSTR(lang.as_ptr())).unwrap_or_default();
                    if !supported.as_bool() {
                        continue;
                    }
                    if let Ok(checker) = factory.CreateSpellChecker(PCWSTR(lang.as_ptr())) {
                        out.push(checker);
                        break;
                    }
                }
            }
        }
        out
    }

    /// Wywołuje `f` z listą sprawdzaczy - pustą, gdy system nie ma
    /// zainstalowanego żadnego z obsługiwanych języków.
    fn with_checkers<T>(f: impl FnOnce(&[ISpellChecker]) -> T) -> T {
        CHECKERS.with(|cell| {
            let mut slot = cell.borrow_mut();
            f(slot.get_or_insert_with(create))
        })
    }

    pub fn available() -> bool {
        with_checkers(|c| !c.is_empty())
    }

    /// Błędy zgłoszone przez pojedynczy słownik.
    fn errors_of(checker: &ISpellChecker, text: &HSTRING) -> Vec<SpellError> {
        unsafe {
            let Ok(errors) = checker.Check(PCWSTR(text.as_ptr())) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            // Enumerator sygnalizuje koniec, zwracając S_FALSE i pusty wskaźnik,
            // a nie błąd - stąd warunek na `None`.
            loop {
                let mut slot: Option<windows::Win32::Globalization::ISpellingError> = None;
                if errors.Next(&mut slot).is_err() {
                    break;
                }
                let Some(item) = slot else { break };
                let (Ok(start), Ok(length)) = (item.StartIndex(), item.Length()) else {
                    continue;
                };
                let action = item.CorrectiveAction().unwrap_or(CORRECTIVE_ACTION_GET_SUGGESTIONS);
                let replacement = if action == CORRECTIVE_ACTION_REPLACE {
                    item.Replacement().ok().and_then(|p| p.to_string().ok())
                } else {
                    None
                };
                out.push(SpellError { start, length, replacement });
            }
            out
        }
    }

    /// Czy dany słownik zna to słowo. Pytamy o pojedyncze słowo zamiast
    /// porównywać zakresy błędów, bo tokenizacja bywa zależna od języka.
    fn knows(checker: &ISpellChecker, word: &str) -> bool {
        let wide = HSTRING::from(word);
        errors_of(checker, &wide).is_empty()
    }

    pub fn check(text: &str) -> Vec<SpellError> {
        let wide = HSTRING::from(text);
        with_checkers(|checkers| {
            let Some((primary, rest)) = checkers.split_first() else {
                return Vec::new();
            };
            let mut out = errors_of(primary, &wide);
            if rest.is_empty() || out.is_empty() {
                return out;
            }
            // To samo słowo powtórzone w tekście pytamy tylko raz.
            let units: Vec<u16> = text.encode_utf16().collect();
            let mut cache: HashMap<String, bool> = HashMap::new();
            out.retain(|e| {
                let (from, to) = (e.start as usize, (e.start + e.length) as usize);
                let Some(slice) = units.get(from..to) else {
                    return true;
                };
                let word = String::from_utf16_lossy(slice);
                let known = *cache
                    .entry(word.clone())
                    .or_insert_with(|| rest.iter().any(|c| knows(c, &word)));
                !known
            });
            out
        })
    }

    /// Odległość edycyjna - potrzebna, by wymieszać podpowiedzi z dwóch
    /// słowników sensownie. Bez tego pierwszy słownik zajmuje całą listę
    /// swoimi wariantami, choć literówka jest oczywiście z drugiego języka.
    fn distance(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.to_lowercase().chars().collect();
        let b: Vec<char> = b.to_lowercase().chars().collect();
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        let mut row = vec![0usize; b.len() + 1];
        for (i, ca) in a.iter().enumerate() {
            row[0] = i + 1;
            for (j, cb) in b.iter().enumerate() {
                let cost = if ca == cb { 0 } else { 1 };
                row[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(row[j] + 1);
            }
            std::mem::swap(&mut prev, &mut row);
        }
        prev[b.len()]
    }

    pub fn suggest(word: &str) -> Vec<String> {
        let wide = HSTRING::from(word);
        with_checkers(|checkers| {
            // Zbieramy z obu słowników, potem układamy wg podobieństwa do
            // wpisanego słowa - sortowanie stabilne, więc przy remisie zostaje
            // kolejność systemowa (i pierwszeństwo polskiego).
            let mut out: Vec<String> = Vec::new();
            for checker in checkers {
                let mut taken = 0;
                unsafe {
                    let Ok(list) = checker.Suggest(PCWSTR(wide.as_ptr())) else {
                        continue;
                    };
                    let mut buffer = [windows::core::PWSTR::null(); 1];
                    let mut fetched = 0u32;
                    while list.Next(&mut buffer, Some(&mut fetched)).is_ok() && fetched == 1 {
                        if buffer[0].is_null() {
                            break;
                        }
                        if let Ok(text) = buffer[0].to_string() {
                            if !out.contains(&text) {
                                out.push(text);
                            }
                        }
                        windows::Win32::System::Com::CoTaskMemFree(Some(buffer[0].0 as *const _));
                        buffer[0] = windows::core::PWSTR::null();
                        fetched = 0;
                        taken += 1;
                        if taken >= 8 {
                            break;
                        }
                    }
                }
            }
            out.sort_by_key(|s| distance(word, s));
            out.truncate(8);
            out
        })
    }

    /// Dopisuje słowo do słownika użytkownika - na stałe, także dla innych
    /// programów korzystających z systemowego sprawdzania pisowni. Wystarczy
    /// jeden słownik: słowo znane któremukolwiek przestaje być błędem.
    pub fn add(word: &str) -> Result<()> {
        let wide = HSTRING::from(word);
        with_checkers(|checkers| match checkers.first() {
            Some(checker) => unsafe {
                checker
                    .Add(PCWSTR(wide.as_ptr()))
                    .map_err(|e| AppError::Other(format!("słownik użytkownika: {e}")))
            },
            None => Err(AppError::Other(
                "system nie ma zainstalowanego słownika pisowni".into(),
            )),
        })
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn available() -> bool {
        false
    }
    pub fn check(_text: &str) -> Vec<SpellError> {
        Vec::new()
    }
    pub fn suggest(_word: &str) -> Vec<String> {
        Vec::new()
    }
    pub fn add(_word: &str) -> Result<()> {
        Err(AppError::Other("sprawdzanie pisowni tylko na Windows".into()))
    }
}

#[tauri::command]
pub fn spell_available() -> bool {
    imp::available()
}

#[tauri::command]
pub fn spell_check(text: String) -> Vec<SpellError> {
    imp::check(&text)
}

#[tauri::command]
pub fn spell_suggest(word: String) -> Vec<String> {
    imp::suggest(&word)
}

#[tauri::command]
pub fn spell_add(word: String) -> Result<()> {
    imp::add(&word)
}
