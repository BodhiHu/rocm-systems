//! Filesystem paths for the Mirage daemon, obeying the
//! [XDG Base Directory Specification].
//!
//! [XDG Base Directory Specification]:
//!     https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html

use std::env;
use std::path::{Path, PathBuf};

/// Application name used as the subdirectory under each XDG base directory.
const APP_DIR: &str = "mirage";

/// Filename of the daemon's Unix domain socket.
const SOCKET_FILE: &str = "mirage.sock";

/// Filename of the daemon's configuration file.
const CONFIG_FILE: &str = "config.mcfg";

/// Returns the value of `$HOME`, or `None` if it is unset or empty.
fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
    }

/// Returns the value of an XDG environment variable, but only if it is set to
/// an absolute path (per the XDG spec — relative paths must be ignored).
fn xdg_env(var: &str) -> Option<PathBuf> {
    env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

fn fallback_runtime_dir(uid: libc::uid_t, run_root: &Path, tmp_root: &Path) -> PathBuf {
    let run_user_dir = run_root.join(uid.to_string());
    if run_user_dir.is_dir() {
        return run_user_dir.join(APP_DIR);
    }
    tmp_root.join(format!("{APP_DIR}-{uid}"))
}

/// Returns the directory where Mirage should place its Unix domain socket.
///
/// Resolution order:
/// 1. `$XDG_RUNTIME_DIR/mirage` (if `XDG_RUNTIME_DIR` is set to an absolute
///    path, as required by the XDG spec).
/// 2. `/run/user/<uid>/mirage` when the standard per-user runtime base exists.
/// 3. `/tmp/mirage-<uid>` as a final fallback.
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = xdg_env("XDG_RUNTIME_DIR") {
        return dir.join(APP_DIR);
    }
    // Safe fallback: prefer the standard per-user runtime base, then /tmp.
    // SAFETY: `getuid` is always safe to call; it cannot fail.
    let uid = unsafe { libc::getuid() };
    fallback_runtime_dir(uid, Path::new("/run/user"), Path::new("/tmp"))
}

/// Returns the full path to the daemon's Unix domain socket.
pub fn socket_path() -> PathBuf {
    runtime_dir().join(SOCKET_FILE)
}

/// Returns the directory where Mirage should read/write configuration.
///
/// Resolution order:
/// 1. `$XDG_CONFIG_HOME/mirage` (if `XDG_CONFIG_HOME` is set to an absolute
///    path).
/// 2. `$HOME/.config/mirage` as the XDG-specified default.
/// 3. `./mirage` as a last-resort fallback when `$HOME` is also unset.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = xdg_env("XDG_CONFIG_HOME") {
        return dir.join(APP_DIR);
    }
    if let Some(home) = home_dir() {
        return home.join(".config").join(APP_DIR);
    }
    PathBuf::from(APP_DIR)
}

/// Returns the full path to the daemon's configuration file.
pub fn config_path() -> PathBuf {
    config_dir().join(CONFIG_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        env::temp_dir().join(format!("mirage_schema_{label}_{}_{}", std::process::id(), nanos))
    }

    /// Serialise tests that mutate process-wide environment variables.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn socket_path_uses_xdg_runtime_dir() {
        let _g = env_lock();
        // SAFETY: guarded by `env_lock`.
        unsafe { env::set_var("XDG_RUNTIME_DIR", "/run/user/1000") };
        assert_eq!(socket_path(), PathBuf::from("/run/user/1000/mirage/mirage.sock"));
    }

    #[test]
    fn socket_path_falls_back_when_runtime_dir_missing() {
        let _g = env_lock();
        // SAFETY: guarded by `env_lock`.
        unsafe { env::remove_var("XDG_RUNTIME_DIR") };
        let p = socket_path();
        let uid = unsafe { libc::getuid() };
        let run_user = PathBuf::from(format!("/run/user/{uid}/mirage/mirage.sock"));
        let tmp_fallback = PathBuf::from(format!("/tmp/mirage-{uid}/mirage.sock"));
        assert!(p == run_user || p == tmp_fallback);
    }

    #[test]
    fn socket_path_ignores_relative_runtime_dir() {
        let _g = env_lock();
        // SAFETY: guarded by `env_lock`.
        unsafe { env::set_var("XDG_RUNTIME_DIR", "relative/path") };
        let p = socket_path();
        let uid = unsafe { libc::getuid() };
        let run_user = PathBuf::from(format!("/run/user/{uid}/mirage/mirage.sock"));
        let tmp_fallback = PathBuf::from(format!("/tmp/mirage-{uid}/mirage.sock"));
        assert!(p == run_user || p == tmp_fallback);
    }

    #[test]
    fn runtime_dir_prefers_run_user_when_available() {
        let run_root = unique_temp_dir("run_root");
        let tmp_root = unique_temp_dir("tmp_root");
        let uid: libc::uid_t = 4242;
        let run_user_dir = run_root.join(uid.to_string());

        fs::create_dir_all(&run_user_dir).unwrap();

        assert_eq!(
            fallback_runtime_dir(uid, &run_root, &tmp_root),
            run_user_dir.join(APP_DIR),
        );

        fs::remove_dir_all(&run_root).unwrap();
        let _ = fs::remove_dir_all(&tmp_root);
    }

    #[test]
    fn runtime_dir_falls_back_to_tmp_when_run_user_missing() {
        let run_root = unique_temp_dir("run_root_missing");
        let tmp_root = unique_temp_dir("tmp_root_missing");
        let uid: libc::uid_t = 5252;

        assert_eq!(
            fallback_runtime_dir(uid, &run_root, &tmp_root),
            tmp_root.join(format!("mirage-{uid}")),
        );

        let _ = fs::remove_dir_all(&run_root);
        let _ = fs::remove_dir_all(&tmp_root);
    }

    #[test]
    fn config_path_uses_xdg_config_home() {
        let _g = env_lock();
        // SAFETY: guarded by `env_lock`.
        unsafe { env::set_var("XDG_CONFIG_HOME", "/custom/config") };
        assert_eq!(config_path(), PathBuf::from("/custom/config/mirage/config.mcfg"));
    }

    #[test]
    fn config_path_falls_back_to_home() {
        let _g = env_lock();
        // SAFETY: guarded by `env_lock`.
        unsafe {
            env::remove_var("XDG_CONFIG_HOME");
            env::set_var("HOME", "/home/tester");
        }
        assert_eq!(
            config_path(),
            PathBuf::from("/home/tester/.config/mirage/config.mcfg"),
        );
    }
}
