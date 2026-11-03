// Hero settings as the Rust side stores them (settings.rs). Missing values take the same defaults:
// shown, on the main screen, and standing still unless the user chose to let it walk.
export function heroSettings(value) {
  return {
    show: value?.show !== false,
    screen: typeof value?.screen === 'string' ? value.screen : 'main',
    still: value?.still !== false,
  }
}
