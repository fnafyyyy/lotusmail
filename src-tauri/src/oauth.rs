//! Logowanie OAuth2 do kont Microsoft (Outlook, Microsoft 365).
//!
//! Microsoft wygasza logowanie hasłem do IMAP i SMTP, więc dla tych kont to
//! jedyna droga. Używamy przepływu kodu autoryzacyjnego z PKCE i nasłuchem na
//! pętli zwrotnej - takiego, jaki Microsoft przewiduje dla aplikacji
//! klasycznych: bez sekretu klienta, bo program na komputerze użytkownika
//! nie ma gdzie go bezpiecznie schować.
//!
//! Token dostępowy żyje krótko (około godziny) i trzymamy go tylko w pamięci.
//! Trwały jest token odświeżania i on trafia do pęku kluczy, obok haseł.

use crate::error::{AppError, Result};
use base64::Engine;
use serde::Deserialize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const AUTH_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";

/// Uprawnienia, o które prosimy. `offline_access` daje token odświeżania -
/// bez niego użytkownik logowałby się co godzinę. `openid`/`email` dokładają
/// token tożsamości, z którego czytamy, na jakie konto ktoś się faktycznie
/// zalogował. Dokumentacja pozwala mieszać scope'y OIDC z uprawnieniami
/// jednego zasobu (tu: outlook.office.com) i tak właśnie to wygląda.
const SCOPES: &str = "openid email offline_access https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send";

/// Ile czasu przed wygaśnięciem uznajemy token za nieświeży. Zapas chroni
/// przed sytuacją, w której token wygasa w trakcie długiej synchronizacji.
const EXPIRY_MARGIN: u64 = 120;

#[derive(Debug, Clone)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix epoch, kiedy token dostępowy przestaje być ważny.
    pub expires_at: u64,
    /// Adres konta, na które faktycznie się zalogowano (z tokenu tożsamości).
    /// Puste, jeśli serwer go nie przysłał.
    pub account: Option<String>,
}

impl Tokens {
    pub fn is_fresh(&self) -> bool {
        now() + EXPIRY_MARGIN < self.expires_at
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    /// Microsoft zwraca nowy token odświeżania przy każdym odświeżeniu,
    /// ale nie jest to gwarantowane - stąd `Option`.
    refresh_token: Option<String>,
    expires_in: u64,
    /// Przychodzi tylko wtedy, gdy poprosiliśmy o `openid`.
    id_token: Option<String>,
}

/// Adres konta wyjęty z tokenu tożsamości.
///
/// Podpisu nie sprawdzamy i nie musimy: token przyszedł prosto z serwera
/// Microsoftu po TLS, jest wystawiony na naszą aplikację, a my używamy go
/// wyłącznie do nazwania konta - nie do żadnej decyzji o dostępie.
/// Dokumentacja przestrzega przed czytaniem tokenów cudzych API; ten jest nasz.
fn account_of(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    ["preferred_username", "email", "upn"]
        .into_iter()
        .filter_map(|key| claims.get(key).and_then(|v| v.as_str()))
        .find(|value| value.contains('@'))
        .map(|value| value.to_string())
}

/// Losowy ciąg dla PKCE. Weryfikator musi mieć 43-128 znaków z bezpiecznego
/// alfabetu; bierzemy 32 bajty losowe i kodujemy base64url.
fn random_verifier() -> String {
    use chacha20poly1305::aead::rand_core::RngCore;
    use chacha20poly1305::aead::OsRng;
    let mut raw = [0u8; 32];
    OsRng.fill_bytes(&mut raw);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

fn challenge_of(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Prosty kodujący adres URL - w zapytaniu idą adresy i lista uprawnień
/// ze spacjami, więc nie da się ich wkleić wprost.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Odwrotność `urlencode`. Parametry wracają z przeglądarki zakodowane -
/// bez rozkodowania `error_description` byłby ciągiem `%20`, a kod
/// autoryzacyjny ze znakiem specjalnym poleciałby do Microsoftu zepsuty
/// i wrócił jako `invalid_grant`.
fn urldecode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&value[i + 1..i + 3], 16) {
                Ok(byte) => {
                    out.push(byte);
                    i += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Odbiera kod autoryzacyjny na pętli zwrotnej.
///
/// Port bierzemy od systemu (`127.0.0.1:0`), bo rejestracja aplikacji
/// u Microsoftu dopuszcza dowolny port dla `http://localhost` - dzięki temu
/// nic nie trzeba wpisywać na sztywno ani martwić się o zajęty port.
async fn wait_for_code(listener: tokio::net::TcpListener, state: &str) -> Result<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    loop {
        let (stream, _) = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| AppError::Other("logowanie przerwane - minęło 5 minut".into()))??;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = BufReader::new(read_half).lines();
        // Przeglądarki otwierają połączenia „na zapas" i dopytują o faviconkę.
        // Pierwsze z brzegu połączenie nie musi więc nieść przekierowania -
        // wcześniej takie puste gniazdo zajmowało nasłuch aż do limitu czasu,
        // a prawdziwy powrót z logowania nie miał gdzie wejść.
        let request = match tokio::time::timeout(Duration::from_secs(20), lines.next_line()).await {
            Ok(Ok(Some(line))) => line,
            _ => continue,
        };

        // "GET /?code=...&state=... HTTP/1.1"
        let target = request.split_whitespace().nth(1).unwrap_or("");
        let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
        let mut code = None;
        let mut got_state = None;
        let mut error = None;
        let mut detail = None;
        for pair in query.split('&') {
            match pair.split_once('=') {
                Some(("code", v)) => code = Some(urldecode(v)),
                Some(("state", v)) => got_state = Some(urldecode(v)),
                Some(("error", v)) => error = Some(urldecode(v)),
                Some(("error_description", v)) => detail = Some(urldecode(v)),
                _ => {}
            }
        }

        if code.is_none() && error.is_none() {
            let _ = write_half
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
            continue;
        }

        let ok = code.is_some() && got_state.as_deref() == Some(state);
        let body = if ok {
            "<h2>Zalogowano</h2><p>Możesz wrócić do lotusMaila.</p>"
        } else {
            "<h2>Nie udało się</h2><p>Wróć do lotusMaila i spróbuj ponownie.</p>"
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        write_half.write_all(response.as_bytes()).await?;
        write_half.flush().await?;

        if let Some(e) = error {
            // `error` to sam kod („access_denied"), całą treść niesie
            // `error_description` - bez niego zostawaliśmy z komunikatem,
            // z którego nic nie wynika.
            let detail = detail.unwrap_or_default();
            let detail = detail.lines().next().unwrap_or("").trim().to_string();
            return Err(AppError::Other(if detail.is_empty() {
                format!("Microsoft odmówił logowania: {e}")
            } else {
                format!("Microsoft odmówił logowania ({e}): {detail}")
            }));
        }
        // Kontrola `state` chroni przed podrzuceniem cudzego kodu na nasz nasłuch.
        if got_state.as_deref() != Some(state) {
            return Err(AppError::Other(
                "odpowiedź nie pasuje do rozpoczętego logowania".into(),
            ));
        }
        return code.ok_or_else(|| AppError::Other("brak kodu autoryzacyjnego".into()));
    }
}

async fn exchange(params: Vec<(&str, String)>) -> Result<Tokens> {
    let client = reqwest::Client::new();
    let response = client
        .post(TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| AppError::Other(format!("połączenie z Microsoftem: {e}")))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| AppError::Other(format!("odpowiedź Microsoftu: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "Microsoft odrzucił żądanie tokenu ({status}): {}",
            text.chars().take(300).collect::<String>()
        )));
    }
    let parsed: TokenResponse = serde_json::from_str(&text)
        .map_err(|e| AppError::Other(format!("nieczytelna odpowiedź tokenu: {e}")))?;
    Ok(Tokens {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token.unwrap_or_default(),
        expires_at: now() + parsed.expires_in,
        account: parsed.id_token.as_deref().and_then(account_of),
    })
}

/// Pełne logowanie: otwiera przeglądarkę i czeka na powrót z kodem.
/// Zwraca komplet tokenów razem z adresem konta wybranym przez użytkownika.
pub async fn login(client_id: &str) -> Result<Tokens> {
    if client_id.trim().is_empty() {
        return Err(AppError::Other(
            "nie ustawiono identyfikatora aplikacji Microsoft".into(),
        ));
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let redirect = format!("http://localhost:{port}");

    let verifier = random_verifier();
    let challenge = challenge_of(&verifier);
    let state = random_verifier();

    let url = format!(
        "{AUTH_URL}?client_id={}&response_type=code&redirect_uri={}&response_mode=query\
         &scope={}&state={}&code_challenge={}&code_challenge_method=S256&prompt=select_account",
        urlencode(client_id),
        urlencode(&redirect),
        urlencode(SCOPES),
        urlencode(&state),
        urlencode(&challenge),
    );
    open::that(&url).map_err(|e| AppError::Other(format!("nie udało się otworzyć przeglądarki: {e}")))?;

    let code = wait_for_code(listener, &state).await?;
    exchange(vec![
        ("client_id", client_id.to_string()),
        ("grant_type", "authorization_code".into()),
        ("code", code),
        ("redirect_uri", redirect),
        ("code_verifier", verifier),
        ("scope", SCOPES.to_string()),
    ])
    .await
}

/// Wymienia token odświeżania na świeży token dostępowy.
pub async fn refresh(client_id: &str, refresh_token: &str) -> Result<Tokens> {
    let mut tokens = exchange(vec![
        ("client_id", client_id.to_string()),
        ("grant_type", "refresh_token".into()),
        ("refresh_token", refresh_token.to_string()),
        ("scope", SCOPES.to_string()),
    ])
    .await
    // Token odświeżania bywa cofnięty (zmiana hasła, wylogowanie wszystkich
    // urządzeń, polityka firmy). Dokumentacja przewiduje na to `invalid_grant`
    // i jedyne wyjście to ponowne logowanie - lepiej powiedzieć to wprost niż
    // pokazywać surową odpowiedź serwera.
    .map_err(|e| {
        let text = e.to_string();
        if text.contains("invalid_grant") || text.contains("interaction_required") {
            AppError::Other(
                "dostęp do konta Microsoft wygasł lub został cofnięty - zaloguj się ponownie"
                    .into(),
            )
        } else {
            e
        }
    })?;
    // Microsoft nie zawsze odsyła nowy token odświeżania - wtedy zostaje stary.
    if tokens.refresh_token.is_empty() {
        tokens.refresh_token = refresh_token.to_string();
    }
    Ok(tokens)
}

/// The XOAUTH2 SASL string, unencoded. IMAP and SMTP both accept this shape.
///
/// Kept separate from the base64 form because async-imap encodes the
/// authenticator's answer itself - handing it something already encoded would
/// send it doubly wrapped and the server would reject the login.
pub fn sasl_raw(user: &str, access_token: &str) -> String {
    format!("user={user}\x01auth=Bearer {access_token}\x01\x01")
}

/// The same string base64-encoded, for callers that need it pre-wrapped.
pub fn xoauth2(user: &str, access_token: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(sasl_raw(user, access_token))
}

#[cfg(test)]
mod testy {
    use super::*;

    #[test]
    fn weryfikator_ma_wlasciwa_dlugosc() {
        let v = random_verifier();
        assert!((43..=128).contains(&v.len()), "długość {}", v.len());
    }

    #[test]
    fn wyzwanie_jest_powtarzalne() {
        assert_eq!(challenge_of("abc"), challenge_of("abc"));
        assert_ne!(challenge_of("abc"), challenge_of("abd"));
    }

    #[test]
    fn dekodowanie_parametrow_zwrotnych() {
        assert_eq!(urldecode("AwABAA%2FbC%2Bd"), "AwABAA/bC+d");
        assert_eq!(urldecode("the+user+canceled"), "the user canceled");
        assert_eq!(urldecode("AADSTS65004%3A+odmowa"), "AADSTS65004: odmowa");
        // Uszkodzony ogon nie może wywrócić logowania.
        assert_eq!(urldecode("abc%"), "abc%");
    }

    #[test]
    fn adres_konta_z_tokenu_tozsamosci() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"preferred_username":"ktos@firma.pl","name":"Ktos"}"#);
        let token = format!("naglowek.{payload}.podpis");
        assert_eq!(account_of(&token).as_deref(), Some("ktos@firma.pl"));
        assert_eq!(account_of("bezsensu"), None);
    }

    #[test]
    fn ciag_xoauth2_ma_format_z_dokumentacji() {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(xoauth2("a@b.pl", "TOKEN"))
            .expect("base64");
        assert_eq!(String::from_utf8(raw).unwrap(), "user=a@b.pl\x01auth=Bearer TOKEN\x01\x01");
    }
}
