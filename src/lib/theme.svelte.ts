// Współdzielony stan motywu - czyta go Sidebar (przełącznik) i ReadingPane
// (dobór tła treści maila). Domyślnie ciemny.
export const theme = $state({
  dark: localStorage.getItem("motyw") !== "jasny",
});
