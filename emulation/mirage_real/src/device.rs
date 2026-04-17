use std::fs::OpenOptions;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const KFD_DEVICE_PATH: &str = "/dev/kfd";
const DRI_DIR: &str = "/dev/dri";

/// Handle to the real hardware.
pub struct RealEmulator {
    pub(crate) kfd: OwnedFd,
    /// DRM render nodes, keyed by minor number so the GPU index is stable
    /// across calls. Lazily populated on first access. A `Mutex` is fine
    /// here — these are not hot paths compared to ioctl round-trips.
    render_nodes: Mutex<Vec<RenderNode>>,
}

struct RenderNode {
    path: PathBuf,
    #[allow(dead_code)] // held open to keep the kernel-side fd alive
    fd: OwnedFd,
}

impl std::fmt::Debug for RealEmulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealEmulator")
            .field("kfd_fd", &self.kfd.as_raw_fd())
            .finish_non_exhaustive()
    }
}

impl RealEmulator {
    /// Return `true` if this host has a KFD device node at all, meaning
    /// [`RealEmulator::detect`] has a chance of succeeding.
    pub fn hardware_available() -> bool {
        Path::new(KFD_DEVICE_PATH).exists()
    }

    /// Open `/dev/kfd` and enumerate `/dev/dri/renderD*`. Returns `None`
    /// if no KFD device is present, otherwise propagates the underlying
    /// `io::Error`.
    pub fn detect() -> io::Result<Option<Self>> {
        if !Self::hardware_available() {
            return Ok(None);
        }
        let kfd = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(KFD_DEVICE_PATH)?;
        let render_nodes = Self::open_render_nodes();
        Ok(Some(Self {
            kfd: kfd.into(),
            render_nodes: Mutex::new(render_nodes),
        }))
    }

    fn open_render_nodes() -> Vec<RenderNode> {
        let mut nodes = Vec::new();
        let Ok(dir) = std::fs::read_dir(DRI_DIR) else {
            return nodes;
        };
        let mut entries: Vec<_> = dir
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
            .collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if let Ok(fd) = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_CLOEXEC)
                .open(entry.path())
            {
                nodes.push(RenderNode {
                    path: entry.path(),
                    fd: fd.into(),
                });
            }
        }
        nodes
    }

    /// List of DRM render-node paths successfully opened.
    pub fn render_node_paths(&self) -> Vec<PathBuf> {
        self.render_nodes
            .lock()
            .unwrap()
            .iter()
            .map(|node| node.path.clone())
            .collect()
    }

    pub(crate) fn primary_render_fd(&self) -> io::Result<i32> {
        self.render_nodes
            .lock()
            .unwrap()
            .first()
            .map(|node| node.fd.as_raw_fd())
            .ok_or_else(|| io::Error::from_raw_os_error(libc::ENODEV))
    }
}
