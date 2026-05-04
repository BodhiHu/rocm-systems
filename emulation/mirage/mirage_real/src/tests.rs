use crate::RealEmulator;

#[test]
fn detects_hardware_presence_without_panicking() {
    // Whatever the answer, this must not crash.
    let _ = RealEmulator::hardware_available();
}

#[test]
fn detect_returns_none_without_kfd() {
    if RealEmulator::hardware_available() {
        // skip on hosts with real hardware
        return;
    }
    assert!(RealEmulator::detect().unwrap().is_none());
}

#[test]
fn get_version_succeeds_when_hardware_present() {
    let Some(emu) = RealEmulator::detect().unwrap() else {
        eprintln!("no /dev/kfd; skipping");
        return;
    };
    let resp = emu.kfd_get_version().expect("get_version on real hw");
    assert!(resp.major_version >= 1, "kfd reports version {resp:?}");
}
