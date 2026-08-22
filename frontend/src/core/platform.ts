// Modifier-key display label: macOS conventionally shows Cmd (⌘), other
// platforms show Ctrl. Keydown handlers themselves always accept both
// (`e.ctrlKey || e.metaKey`) regardless of platform - this only picks which
// one to display in tooltips/hints.
export const MOD_KEY = /Mac|iPod|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl';
