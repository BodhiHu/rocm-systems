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

    /// Return the raw fd of the render node at the given index.
    pub(crate) fn render_fd_by_index(&self, index: usize) -> io::Result<i32> {
        self.render_nodes
            .lock()
            .unwrap()
            .get(index)
            .map(|node| node.fd.as_raw_fd())
            .ok_or_else(|| io::Error::from_raw_os_error(libc::ENODEV))
    }

    /// Return the number of render nodes.
    pub(crate) fn render_node_count(&self) -> usize {
        self.render_nodes.lock().unwrap().len()
    }

    /// Find the host render fd for a given KFD gpu_id by reading
    /// the sysfs topology to find the drm_render_minor and then
    /// matching it to our open render nodes.
    pub(crate) fn find_render_fd_for_gpu(&self, gpu_id: u32) -> Option<i32> {
        // Walk /sys/class/kfd/kfd/topology/nodes/*/
        let topo = std::path::Path::new("/sys/class/kfd/kfd/topology/nodes");
        let dir = std::fs::read_dir(topo).ok()?;
        for entry in dir.flatten() {
            let props_path = entry.path().join("properties");
            let gpu_id_path = entry.path().join("gpu_id");
            // Read gpu_id for this node.
            let node_gpu_id: u32 = std::fs::read_to_string(&gpu_id_path)
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            if node_gpu_id != gpu_id {
                continue;
            }
            // Found the node; read drm_render_minor from properties.
            let props = std::fs::read_to_string(&props_path).ok()?;
            for line in props.lines() {
                if let Some(rest) = line.strip_prefix("drm_render_minor ") {
                    if let Ok(minor) = rest.trim().parse::<u32>() {
                        let target = format!("renderD{minor}");
                        let nodes = self.render_nodes.lock().unwrap();
                        for node in nodes.iter() {
                            if node
                                .path
                                .file_name()
                                .map_or(false, |n| n == target.as_str())
                            {
                                return Some(node.fd.as_raw_fd());
                            }
                        }
                    }
                }
            }
        }
        None
    }
}
