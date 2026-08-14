-- Dane przykładowe: dwa konta (praca + prywatne), żeby pokazać zunifikowaną
-- skrzynkę, zakładki Smart Inbox, snooze i resztę interfejsu bez konfiguracji.

INSERT INTO accounts (id, email, display_name, imap_host, imap_port, smtp_host, smtp_port, auth_kind) VALUES
    (1, 'demo.praca@example.com',    'Praca (demo)',    '', 993, '', 587, 'demo'),
    (2, 'demo.prywatne@example.com', 'Prywatne (demo)', '', 993, '', 587, 'demo');

INSERT INTO folders (id, account_id, name, display_name, kind) VALUES
    (1, 1, 'INBOX',   'Odebrane',      'inbox'),
    (2, 1, 'Sent',    'Wysłane',       'sent'),
    (3, 1, 'Archive', 'Archiwum',      'archive'),
    (4, 1, 'Trash',   'Kosz',          'trash'),
    (5, 2, 'INBOX',   'Odebrane',      'inbox'),
    (6, 2, 'Sent',    'Wysłane',       'sent'),
    (7, 2, 'Trash',   'Kosz',          'trash');

INSERT INTO messages (folder_id, uid, subject, from_name, from_addr, to_addrs, date, preview, is_read, is_flagged, has_attachments, category, snoozed_until) VALUES
    (1, 101, 'Harmonogram wdrożenia na przyszły tydzień', 'Marek Nowicki', 'm.nowicki@example.com', 'demo.praca@example.com',
     unixepoch() - 1800, 'Cześć, przesyłam zaktualizowany harmonogram wdrożenia. Kluczowa zmiana: migracja bazy przesunięta na środę...',
     0, 1, 1, 'primary', NULL),
    (1, 102, 'Re: budżet Q3 - potrzebna Twoja akceptacja', 'Anna Wiśniewska', 'a.wisniewska@example.com', 'demo.praca@example.com',
     unixepoch() - 7200, 'Wracam do tematu budżetu. Dyrekcja czeka na Twoją akceptację do piątku. W załączniku zestawienie...',
     0, 0, 0, 'primary', NULL),
    (1, 103, 'Faktura FV/2026/08/114 za usługi serwerowe', 'System fakturowania', 'noreply@hosting.example.com', 'demo.praca@example.com',
     unixepoch() - 14400, 'Dziękujemy za terminową płatność. Faktura za sierpień jest dostępna w panelu klienta...',
     1, 0, 1, 'notifications', NULL),
    (1, 104, 'Zaproszenie: przegląd sprintu, piątek 10:00', 'Katarzyna Zielińska', 'k.zielinska@example.com', 'demo.praca@example.com',
     unixepoch() - 86400, 'Zapraszam na przegląd sprintu w piątek o 10:00 w sali B. Agenda: demo nowych funkcji, retrospektywa...',
     1, 0, 0, 'primary', NULL),
    (1, 105, 'Alert: użycie dysku na serwerze prod-02 przekroczyło 85%', 'Monitoring', 'alerts@monitoring.example.com', 'demo.praca@example.com',
     unixepoch() - 90000, 'Automatyczne powiadomienie: partycja /var/log na prod-02 osiągnęła 85% pojemności...',
     1, 0, 0, 'notifications', NULL),
    (1, 106, 'Oferta współpracy - odezwę się w przyszłym tygodniu', 'Tomasz Lis', 't.lis@example.com', 'demo.praca@example.com',
     unixepoch() - 172800, 'Dzień dobry, wracając do naszej rozmowy z konferencji - chętnie omówię szczegóły współpracy...',
     1, 0, 0, 'primary', unixepoch() + 259200),
    (5, 201, 'Twoje zamówienie #48291 zostało wysłane', 'Sklep TechZone', 'notifications@techzone.example.com', 'demo.prywatne@example.com',
     unixepoch() - 3600, 'Dobra wiadomość! Twoja paczka jest w drodze. Przewidywana dostawa: jutro do 18:00. Numer śledzenia...',
     0, 0, 0, 'notifications', NULL),
    (5, 202, 'Weekend w górach - rezerwujemy?', 'Michał Pachura', 'michal.p@example.com', 'demo.prywatne@example.com',
     unixepoch() - 10800, 'Hej! Znalazłem fajny pensjonat w Szczyrku na ten długi weekend. Ceny jeszcze normalne, ale trzeba...',
     0, 0, 0, 'primary', NULL),
    (5, 203, 'Tygodnik technologiczny: Rust 2.0 coraz bliżej?', 'Newsletter DevWeekly', 'news@devweekly.example.com', 'demo.prywatne@example.com',
     unixepoch() - 43200, 'W tym tygodniu: co wiemy o planach na Rust 2.0, nowości w Tauri 2.x, benchmark silników SQLite...',
     1, 0, 0, 'newsletters', NULL),
    (5, 204, 'Potwierdzenie wizyty - przegląd samochodu 12.08', 'Serwis AutoMax', 'no-reply@automax.example.com', 'demo.prywatne@example.com',
     unixepoch() - 129600, 'Potwierdzamy termin przeglądu: 12 sierpnia, godz. 9:00. Przewidywany czas: około 2 godzin...',
     1, 0, 0, 'notifications', NULL),
    (5, 205, 'Zdjęcia z urodzin babci', 'Magda Pachura', 'magda.p@example.com', 'demo.prywatne@example.com',
     unixepoch() - 259200, 'Wrzucam zdjęcia z soboty, wyszły świetnie! Babcia bardzo się ucieszyła z tortu...',
     1, 1, 1, 'primary', NULL);

INSERT INTO message_bodies (message_id, html, text)
SELECT id,
    '<div><p>' || preview || '</p><p>To jest przykładowa treść wiadomości demo. Po podłączeniu prawdziwego konta w tym miejscu zobaczysz zsanityzowany HTML maila - bez skryptów i bez zdalnych obrazków śledzących.</p><p>Pozdrawiam,<br>' || from_name || '</p></div>',
    preview || CHAR(10) || CHAR(10) || 'To jest przykładowa treść wiadomości demo.'
FROM messages;
