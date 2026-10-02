//! Jedna instancja programu i twarde zakończenie procesu.
//!
//! Oba mechanizmy bronią przed tym samym wypadkiem: proces po zamknięciu okna
//! zawisł w `ExitProcess` (został mu sam wątek główny), ale dalej trzymał swój
//! silnik WebView2. Kolejne uruchomienie podpięło się do tego osieroconego
//! silnika - ten sam katalog danych - i okno rysowało się, lecz nie reagowało
//! na kliknięcia.
//!
//! Drugiej instancji szukamy po **oknie**, nie po nazwanym muteksie (jak robi
//! to `tauri-plugin-single-instance`): zawieszony proces bez okna trzymałby
//! muteks do restartu systemu i programu nie dałoby się w ogóle uruchomić.
//! Porównanie ścieżki pliku wykonywalnego pozwala też wersji deweloperskiej
//! działać obok zainstalowanej - obie nazywają się `LotusMail.exe`.

#[cfg(windows)]
mod imp {
    use windows::core::{BOOL, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW,
        TerminateProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    /// Klasa głównego okna nadawana przez tao.
    const WINDOW_CLASS: &str = "Tauri Window";

    struct Search {
        own_pid: u32,
        own_exe: String,
        found: Option<HWND>,
    }

    fn exe_of(pid: u32) -> Option<String> {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let res = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            );
            let _ = CloseHandle(process);
            res.ok()?;
            Some(String::from_utf16_lossy(&buf[..len as usize]).to_lowercase())
        }
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == search.own_pid || !IsWindowVisible(hwnd).as_bool() {
            return true.into();
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut class) as usize;
        if String::from_utf16_lossy(&class[..n]) != WINDOW_CLASS {
            return true.into();
        }
        if exe_of(pid).as_deref() != Some(search.own_exe.as_str()) {
            return true.into();
        }
        search.found = Some(hwnd);
        false.into()
    }

    pub fn focus_existing() -> bool {
        let Ok(exe) = std::env::current_exe() else { return false };
        let mut search = Search {
            own_pid: unsafe { GetCurrentProcessId() },
            own_exe: exe.to_string_lossy().to_lowercase(),
            found: None,
        };
        unsafe {
            // Przerwanie wyliczania po trafieniu wraca jako błąd - to nie awaria.
            let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize));
            let Some(hwnd) = search.found else { return false };
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
        true
    }

    pub fn hard_exit() {
        unsafe {
            let _ = TerminateProcess(GetCurrentProcess(), 0);
        }
    }
}

/// Jeśli ten sam plik wykonywalny ma już otwarte okno, wyciąga je na wierzch
/// i zwraca `true` - wołający powinien wtedy zakończyć start.
pub fn focus_existing() -> bool {
    #[cfg(windows)]
    {
        imp::focus_existing()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Kończy proces z pominięciem sprzątania bibliotek (`DLL_PROCESS_DETACH`),
/// w którym zwykłe wyjście potrafiło zawisnąć. Wołane po zamknięciu okien;
/// baza jest w trybie WAL, więc nagłe ucięcie procesu jej nie szkodzi.
pub fn hard_exit() {
    #[cfg(windows)]
    imp::hard_exit();
}
