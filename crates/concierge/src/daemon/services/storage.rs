//! The `/data` btrfs pool: disk enumeration, pool status, and the
//! add/remove/grow mutations, backed by sysfs reads and `btrfs(8)`
//! shell-outs -- no udisks2/D-Bus dependency, matching how `system.rs`
//! already reads `/proc` directly.
//!
//! **Reads come from sysfs where sysfs actually has the data.** Disk
//! enumeration (`disks()`) and the pool's real used-space figure (`pool()`,
//! via the `allocation/*/bytes_used` counters -- `statvfs` lies about free
//! space on btrfs, these don't) are pure sysfs, no command execution.
//!
//! One exception, discovered by inspecting a running system rather than
//! assumed up front: sysfs's `devinfo/<devid>/` directory (missing,
//! in_fs_metadata) is keyed by devid, and `devices/<kname>` is keyed by
//! kernel device name, with no attribute on either side linking the two --
//! there is no pure-sysfs devid-to-path map. `btrfs filesystem show --raw`
//! is the only source for that join (also the only source for a device's
//! individual size/used), so the per-device breakdown in `PoolStatus`
//! parses that command's output instead.
//!
//! **Writes shell out**, mirroring `tls.rs`'s haproxy validation: absolute
//! path, check `status.success()`, fold `stderr` into the error.
//! `add_disk`/`remove_disk` use `tokio::process::Command` (not
//! `std::process`, unlike `tls.rs`'s synchronous haproxy check) because
//! `btrfs device remove` runs for minutes, not milliseconds.
//!
//! **Safety.** A caller-supplied device string is never passed to `btrfs`
//! directly: `add_disk`/`remove_disk` re-enumerate with `disks()` and act on
//! the path from that enumeration, the same discipline
//! `managed_services.rs` applies to config paths.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use concierge_api::{DiskInfo, DiskRole, PoolDevice, PoolOperation, PoolOperationKind, PoolOperationState, PoolStatus};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::Mutex as AsyncMutex;

use super::{Result, ServiceError, StorageService};

/// btrfs-backed `/data` pool, with in-progress add/remove operations
/// tracked in memory rather than a job framework -- there is only ever at
/// most one at a time (a second mutation while one is running is rejected).
pub struct BtrfsStorageService {
    data_mount_point: PathBuf,
    btrfs_bin: PathBuf,
    operation: Arc<AsyncMutex<Option<PoolOperation>>>,
}

impl BtrfsStorageService {
    pub fn new(data_mount_point: PathBuf, btrfs_bin: PathBuf) -> Self {
        Self { data_mount_point, btrfs_bin, operation: Arc::new(AsyncMutex::new(None)) }
    }

    fn fsid(&self) -> Result<String> {
        resolve_fsid(&self.data_mount_point).ok_or_else(|| {
            ServiceError::Other(anyhow::anyhow!(
                "{} is not a mounted btrfs filesystem",
                self.data_mount_point.display()
            ))
        })
    }

    async fn device_breakdown(&self) -> Result<Vec<PoolDevice>> {
        let output = tokio::process::Command::new(&self.btrfs_bin)
            .args(["filesystem", "show", "--raw"])
            .arg(&self.data_mount_point)
            .output()
            .await
            .map_err(io_err)?;
        if !output.status.success() {
            return Err(ServiceError::Other(anyhow::anyhow!(
                "btrfs filesystem show: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(parse_devinfo(&String::from_utf8_lossy(&output.stdout)))
    }

    async fn begin_operation(&self, kind: PoolOperationKind, device: &str) -> Result<()> {
        let mut guard = self.operation.lock().await;
        if matches!(guard.as_ref(), Some(op) if op.state == PoolOperationState::Running) {
            return Err(ServiceError::Conflict(
                "a storage operation is already running".into(),
            ));
        }
        *guard = Some(PoolOperation {
            kind,
            device: device.to_owned(),
            started_at: now_rfc3339(),
            state: PoolOperationState::Running,
        });
        Ok(())
    }

    /// Spawn `args` in the background against `btrfs_bin`, recording the
    /// outcome on `operation` when it finishes. Returns immediately -- the
    /// caller has already recorded the operation as `Running` and returns
    /// the current `PoolStatus` to the API caller without waiting.
    fn spawn_mutation(&self, args: Vec<String>) {
        let btrfs_bin = self.btrfs_bin.clone();
        let operation = self.operation.clone();
        tokio::spawn(async move {
            let result = run_btrfs(&btrfs_bin, &args).await;
            let mut guard = operation.lock().await;
            if let Some(op) = guard.as_mut() {
                op.state = match result {
                    Ok(()) => PoolOperationState::Succeeded,
                    Err(error) => PoolOperationState::Failed { message: error.to_string() },
                };
            }
        });
    }
}

#[async_trait]
impl StorageService for BtrfsStorageService {
    async fn disks(&self) -> Result<Vec<DiskInfo>> {
        enumerate_disks(&self.data_mount_point)
    }

    async fn pool(&self) -> Result<PoolStatus> {
        let fsid = self.fsid()?;
        let used_bytes = read_used_bytes(&fsid);
        let devinfo_degraded = read_devinfo_degraded(&fsid);
        let devices = self.device_breakdown().await?;

        let total_bytes: u64 = devices.iter().map(|device| device.size_bytes).sum();
        let free_bytes = total_bytes.saturating_sub(used_bytes);
        let degraded = devinfo_degraded || devices.iter().any(|device| device.missing);
        let operation = self.operation.lock().await.clone();

        Ok(PoolStatus {
            uuid: fsid,
            mount_point: self.data_mount_point.display().to_string(),
            total_bytes,
            used_bytes,
            free_bytes,
            devices,
            degraded,
            operation,
        })
    }

    async fn add_disk(&self, device: &str, wipe: bool) -> Result<PoolStatus> {
        let disks = enumerate_disks(&self.data_mount_point)?;
        let disk = disks
            .iter()
            .find(|disk| disk.path == device)
            .ok_or_else(|| ServiceError::NotFound(format!("{device}: no such disk")))?;

        match disk.role {
            DiskRole::System => {
                return Err(ServiceError::Conflict(format!(
                    "{device} carries the boot disk and can never be added to the pool"
                )));
            }
            DiskRole::PoolMember => {
                return Err(ServiceError::Conflict(format!("{device} is already a pool member")));
            }
            DiskRole::InUse if !wipe => {
                return Err(ServiceError::Conflict(format!(
                    "{device} already holds a partition table or filesystem; pass --wipe to force"
                )));
            }
            DiskRole::InUse | DiskRole::Available => {}
        }

        self.begin_operation(PoolOperationKind::AddDevice, device).await?;

        let mut args = vec!["device".to_owned(), "add".to_owned()];
        if wipe {
            args.push("-f".to_owned());
        }
        args.push(device.to_owned());
        args.push(self.data_mount_point.display().to_string());
        self.spawn_mutation(args);

        self.pool().await
    }

    async fn remove_disk(&self, device: &str) -> Result<PoolStatus> {
        let disks = enumerate_disks(&self.data_mount_point)?;
        let disk = disks
            .iter()
            .find(|disk| disk.path == device)
            .ok_or_else(|| ServiceError::NotFound(format!("{device}: no such disk")))?;
        if disk.role != DiskRole::PoolMember {
            return Err(ServiceError::Conflict(format!("{device} is not a pool member")));
        }

        let pool = self.pool().await?;
        if !remove_keeps_enough_capacity(&pool.devices, device, pool.used_bytes) {
            return Err(ServiceError::Conflict(format!(
                "removing {device} would leave the pool without enough capacity for the {} bytes already in use",
                pool.used_bytes
            )));
        }

        self.begin_operation(PoolOperationKind::RemoveDevice, device).await?;

        let args = vec![
            "device".to_owned(),
            "remove".to_owned(),
            device.to_owned(),
            self.data_mount_point.display().to_string(),
        ];
        self.spawn_mutation(args);

        self.pool().await
    }

    async fn grow(&self) -> Result<PoolStatus> {
        let fsid = self.fsid()?;
        let devinfo_dir = format!("/sys/fs/btrfs/{fsid}/devinfo");
        let devids = std::fs::read_dir(&devinfo_dir)
            .map_err(io_err)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned());

        let mut failures = Vec::new();
        for devid in devids {
            let args = vec![
                "filesystem".to_owned(),
                "resize".to_owned(),
                format!("{devid}:max"),
                self.data_mount_point.display().to_string(),
            ];
            if let Err(error) = run_btrfs(&self.btrfs_bin, &args).await {
                failures.push(format!("devid {devid}: {error}"));
            }
        }
        if !failures.is_empty() {
            return Err(ServiceError::Other(anyhow::anyhow!(failures.join("; "))));
        }

        self.pool().await
    }
}

// --- disks() ---------------------------------------------------------------

fn enumerate_disks(data_mount_point: &Path) -> Result<Vec<DiskInfo>> {
    let boot_disk = whole_disk_of_root();
    let pool_members = resolve_fsid(data_mount_point)
        .map(pool_member_knames)
        .unwrap_or_default();

    let mut disks = Vec::new();
    for entry in std::fs::read_dir("/sys/class/block").map_err(io_err)? {
        let entry = entry.map_err(io_err)?;
        let kname = entry.file_name().to_string_lossy().into_owned();
        let dev_dir = entry.path();

        // Skip partitions (handled as part of their whole disk) and virtual
        // devices with no `device/` link (loop, dm, md, btrfs itself).
        if dev_dir.join("partition").exists() || !dev_dir.join("device").exists() {
            continue;
        }

        let size_bytes = read_u64(&dev_dir.join("size")).unwrap_or(0) * 512;
        let rotational = read_flag(&dev_dir.join("queue/rotational"));
        let removable = read_flag(&dev_dir.join("removable"));
        let model = read_trimmed(&dev_dir.join("device/model"));
        let serial = read_trimmed(&dev_dir.join("device/serial"));
        let transport = infer_transport(&kname, &dev_dir);
        let has_partitions = has_partition_children(&dev_dir);
        let is_boot_disk = boot_disk.as_deref() == Some(kname.as_str());
        let is_pool_member = is_pool_member_disk(&kname, &dev_dir, &pool_members);

        let role = classify_disk(has_partitions, is_boot_disk, is_pool_member);

        disks.push(DiskInfo {
            path: format!("/dev/{kname}"),
            kname,
            size_bytes,
            model,
            serial,
            transport,
            rotational,
            removable,
            role,
        });
    }
    disks.sort_by(|a, b| a.kname.cmp(&b.kname));
    Ok(disks)
}

/// Pure role assignment: `disks()` precomputes every boolean input from
/// sysfs, so this function itself has no I/O and is trivial to fixture-test.
fn classify_disk(has_partitions: bool, is_boot_disk: bool, is_pool_member: bool) -> DiskRole {
    if is_boot_disk {
        DiskRole::System
    } else if is_pool_member {
        DiskRole::PoolMember
    } else if has_partitions {
        DiskRole::InUse
    } else {
        DiskRole::Available
    }
}

fn has_partition_children(dev_dir: &Path) -> bool {
    std::fs::read_dir(dev_dir)
        .map(|entries| entries.flatten().any(|entry| entry.path().join("partition").exists()))
        .unwrap_or(false)
}

/// A disk is a pool member either directly (an added whole disk, e.g.
/// `vdb`) or through one of its partitions (the original `foyer-data`
/// partition on the boot disk, e.g. `vda7`) -- the latter never actually
/// wins the role assignment since the boot disk is always classified
/// `System` first, but is checked for robustness regardless.
fn is_pool_member_disk(kname: &str, dev_dir: &Path, pool_members: &HashSet<String>) -> bool {
    if pool_members.contains(kname) {
        return true;
    }
    std::fs::read_dir(dev_dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|entry| pool_members.contains(&entry.file_name().to_string_lossy().into_owned()))
        })
        .unwrap_or(false)
}

fn infer_transport(kname: &str, dev_dir: &Path) -> String {
    if kname.starts_with("nvme") {
        return "nvme".to_owned();
    }
    if kname.starts_with("mmcblk") {
        return "mmc".to_owned();
    }
    let subsystem = std::fs::read_link(dev_dir.join("device/subsystem"))
        .ok()
        .and_then(|link| link.to_str().map(str::to_owned))
        .unwrap_or_default();
    if subsystem.contains("/usb") {
        "usb".to_owned()
    } else {
        // SATA/AHCI and virtio-blk both surface as "scsi"/"virtio" here;
        // "ata" is the common case on real hardware and the harmless
        // default for QEMU.
        "ata".to_owned()
    }
}

/// The whole-disk kernel device name (e.g. "vda") carrying the currently
/// mounted root filesystem -- the "System" disk, since Foyer never boots
/// off a disk that doesn't also carry the ESP and the original
/// `foyer-data` partition.
fn whole_disk_of_root() -> Option<String> {
    let kname = mounted_device_kname("/")?;
    if Path::new("/sys/class/block").join(&kname).join("partition").exists() {
        whole_disk_of_partition(&kname)
    } else {
        Some(kname)
    }
}

/// The kernel nests a partition's sysfs directory inside its whole disk's
/// (`/sys/class/block/sda/sda2`, with `/sys/class/block/sda2` a symlink to
/// that same location) -- reading where the symlink actually points gives
/// the parent disk's kernel name without needing udev.
fn whole_disk_of_partition(part_kname: &str) -> Option<String> {
    let link = std::fs::read_link(Path::new("/sys/class/block").join(part_kname)).ok()?;
    let mut components: Vec<_> = link.components().collect();
    components.pop()?; // the partition's own directory name
    let parent = components.pop()?; // the whole disk's directory name
    Some(parent.as_os_str().to_str()?.to_owned())
}

/// The kernel device name backing whatever is mounted at `mount_point`
/// (e.g. "sda7"), resolved via `/proc/self/mountinfo`'s major:minor field
/// and `/sys/dev/block/<major>:<minor>` -- not `/proc/mounts`'s device
/// column plus `canonicalize`, because on a non-initramfs boot the kernel
/// reports the root filesystem's source as the synthetic path `/dev/root`,
/// which has no backing node under `/dev` at all (verified: `readlink -f
/// /dev/root` errors ENOENT on a real Foyer boot). Major:minor plus sysfs
/// has no such gap -- it doesn't go through `/dev` in the first place.
fn mounted_device_kname(mount_point: &str) -> Option<String> {
    let raw = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    let dev_num = parse_mountinfo_devnum(&raw, mount_point)?;
    let link = std::fs::read_link(format!("/sys/dev/block/{dev_num}")).ok()?;
    link.file_name()?.to_str().map(str::to_owned)
}

fn parse_mountinfo_devnum(mountinfo: &str, mount_point: &str) -> Option<String> {
    mountinfo.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        fields.next()?; // mount ID
        fields.next()?; // parent ID
        let dev_num = fields.next()?; // "major:minor"
        fields.next()?; // root
        let mp = fields.next()?;
        (mp == mount_point).then(|| dev_num.to_owned())
    })
}

/// The kernel device name backing `mount_point`, read from
/// `/proc/self/mountinfo`'s mount-source field (the token right after the
/// `- <fstype>` separator) instead of the major:minor field
/// `mounted_device_kname` uses. Needed specifically for a btrfs mount:
/// verified on a real boot that btrfs reports an anonymous internal device
/// number in the major:minor field, not any real member device's -- there
/// is no single "the device" for a filesystem that can span several. The
/// mount-source field is the one place the actual member path used at
/// mount time still shows up. Not used for the *root* filesystem lookup:
/// that field is what broke there instead (the kernel's synthetic
/// "/dev/root" placeholder on a non-initramfs boot has no real backing
/// path at all), which is why that lookup goes through major:minor.
fn mounted_source_kname(mount_point: &str) -> Option<String> {
    let raw = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    let source = parse_mountinfo_source(&raw, mount_point)?;
    Path::new(&source).file_name()?.to_str().map(str::to_owned)
}

fn parse_mountinfo_source(mountinfo: &str, mount_point: &str) -> Option<String> {
    mountinfo.lines().find_map(|line| {
        let (fields, rest) = line.split_once(" - ")?;
        let mut fields = fields.split_whitespace();
        fields.next()?; // mount ID
        fields.next()?; // parent ID
        fields.next()?; // major:minor
        fields.next()?; // root
        let mp = fields.next()?;
        if mp != mount_point {
            return None;
        }
        let mut rest_fields = rest.split_whitespace();
        rest_fields.next()?; // fstype
        rest_fields.next().map(str::to_owned)
    })
}

fn read_u64(path: &Path) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

fn read_flag(path: &Path) -> bool {
    read_trimmed(path).as_deref() == Some("1")
}

fn read_trimmed(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

// --- pool() ------------------------------------------------------------

/// Resolve which mounted btrfs filesystem backs `mount_point`, by finding
/// the fsid whose `devices/` directory (see the module doc for why this
/// directory alone can't give a devid-to-path map) contains the kernel
/// device name backing the mount. procfs plus sysfs only, no command
/// execution, and correctly scoped even with other btrfs filesystems
/// mounted elsewhere on the host (as on a dev machine, unlike the
/// single-purpose appliance).
fn resolve_fsid(mount_point: &Path) -> Option<String> {
    let kname = mounted_source_kname(mount_point.to_str()?)?;

    for entry in std::fs::read_dir("/sys/fs/btrfs").ok()?.flatten() {
        let fsid = entry.file_name().to_string_lossy().into_owned();
        if fsid == "features" {
            continue;
        }
        let devices_dir = entry.path().join("devices");
        let Ok(devices) = std::fs::read_dir(&devices_dir) else { continue };
        if devices
            .flatten()
            .any(|device_entry| device_entry.file_name().to_string_lossy() == kname)
        {
            return Some(fsid);
        }
    }
    None
}

fn pool_member_knames(fsid: String) -> HashSet<String> {
    std::fs::read_dir(format!("/sys/fs/btrfs/{fsid}/devices"))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn read_used_bytes(fsid: &str) -> u64 {
    let base = format!("/sys/fs/btrfs/{fsid}/allocation");
    let readings: Vec<String> = ["data", "metadata", "system"]
        .iter()
        .map(|kind| std::fs::read_to_string(format!("{base}/{kind}/bytes_used")).unwrap_or_default())
        .collect();
    let refs: Vec<&str> = readings.iter().map(String::as_str).collect();
    parse_allocation(&refs)
}

/// Sums the three block-group types' `bytes_used` sysfs counters. Unlike
/// `statvfs` on a btrfs mount, these report real usage.
fn parse_allocation(bytes_used_readings: &[&str]) -> u64 {
    bytes_used_readings
        .iter()
        .filter_map(|raw| raw.trim().parse::<u64>().ok())
        .sum()
}

fn read_devinfo_degraded(fsid: &str) -> bool {
    std::fs::read_dir(format!("/sys/fs/btrfs/{fsid}/devinfo"))
        .map(|entries| {
            entries.flatten().any(|entry| {
                std::fs::read_to_string(entry.path().join("missing"))
                    .map(|content| content.trim() == "1")
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Parses `btrfs filesystem show --raw <mount>` output into per-device
/// records -- the only source for devid-to-path-to-size/used, since sysfs
/// doesn't expose that join (see the module doc). Example line:
/// `\tdevid    1 size 17179869184 used 8589934592 path /dev/vda7`,
/// with a trailing ` MISSING` when the kernel has no live device for that
/// devid.
fn parse_devinfo(output: &str) -> Vec<PoolDevice> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("devid"))
        .filter_map(|line| {
            let missing = line.ends_with("MISSING");
            let mut devid = None;
            let mut size_bytes = None;
            let mut used_bytes = None;
            let mut path = None;

            let mut tokens = line.split_whitespace();
            while let Some(token) = tokens.next() {
                match token {
                    "devid" => devid = tokens.next().and_then(|value| value.parse().ok()),
                    "size" => size_bytes = tokens.next().and_then(|value| value.parse().ok()),
                    "used" => used_bytes = tokens.next().and_then(|value| value.parse().ok()),
                    "path" => path = tokens.next().map(str::to_owned),
                    _ => {}
                }
            }

            Some(PoolDevice {
                devid: devid?,
                path: path.unwrap_or_default(),
                size_bytes: size_bytes.unwrap_or(0),
                used_bytes: used_bytes.unwrap_or(0),
                missing,
            })
        })
        .collect()
}

/// Precondition for `remove_disk`: the devices staying in the pool must
/// have enough raw capacity to hold what's currently in use, since a
/// single/DUP-profile pool has nowhere else to put it (see the module doc
/// on "capacity only" -- no raid1 to fall back on).
fn remove_keeps_enough_capacity(devices: &[PoolDevice], removing: &str, used_bytes: u64) -> bool {
    let remaining: u64 = devices
        .iter()
        .filter(|device| device.path != removing)
        .map(|device| device.size_bytes)
        .sum();
    remaining >= used_bytes
}

// --- shared helpers ------------------------------------------------------

async fn run_btrfs(bin: &Path, args: &[String]) -> Result<()> {
    let output = tokio::process::Command::new(bin)
        .args(args)
        .output()
        .await
        .map_err(io_err)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(ServiceError::Other(anyhow::anyhow!(
            "btrfs {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc().format(&Rfc3339).unwrap_or_default()
}

fn io_err(error: std::io::Error) -> ServiceError {
    ServiceError::Other(error.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real lines captured from `/proc/self/mountinfo` on a booted Foyer
    // image, root= via grub with no initramfs.
    const MOUNTINFO: &str = "\
25 20 0:22 / /efi rw,relatime shared:14 - vfat /dev/sda1 rw,fmask=0022,dmask=0022,codepage=437,iocharset=ascii,shortname=mixed,errors=remount-ro\n\
20 1 8:2 / / ro,relatime shared:1 - ext4 /dev/root ro\n\
180 20 0:59 / /data rw,noatime shared:216 - btrfs /dev/sda7 rw,compress=zstd:1,discard=async,space_cache=v2,subvolid=5,subvol=/\n";

    #[test]
    fn parse_mountinfo_devnum_finds_the_requested_mount_point() {
        assert_eq!(parse_mountinfo_devnum(MOUNTINFO, "/").as_deref(), Some("8:2"));
        assert_eq!(parse_mountinfo_devnum(MOUNTINFO, "/efi").as_deref(), Some("0:22"));
    }

    #[test]
    fn parse_mountinfo_devnum_missing_mount_point() {
        assert_eq!(parse_mountinfo_devnum(MOUNTINFO, "/nope"), None);
    }

    #[test]
    fn parse_mountinfo_source_reads_the_real_member_path_for_btrfs() {
        // The whole point of this parser: /data's major:minor (0:59 above)
        // is btrfs's anonymous internal device number, not a real one --
        // the source field is what still names the actual member device.
        assert_eq!(parse_mountinfo_source(MOUNTINFO, "/data").as_deref(), Some("/dev/sda7"));
    }

    #[test]
    fn parse_mountinfo_source_root_is_the_unusable_placeholder() {
        // Documents why root disk resolution can't use this parser: the
        // source field for / is the kernel's synthetic "/dev/root", which
        // has no backing /dev node on a non-initramfs boot.
        assert_eq!(parse_mountinfo_source(MOUNTINFO, "/").as_deref(), Some("/dev/root"));
    }

    #[test]
    fn classify_disk_boot_disk_is_system_even_if_also_a_pool_member() {
        // The original foyer-data partition lives on the boot disk, so a
        // disk can technically satisfy both checks; System must win.
        assert_eq!(classify_disk(true, true, true), DiskRole::System);
    }

    #[test]
    fn classify_disk_pool_member() {
        assert_eq!(classify_disk(false, false, true), DiskRole::PoolMember);
    }

    #[test]
    fn classify_disk_in_use_blocks_available() {
        assert_eq!(classify_disk(true, false, false), DiskRole::InUse);
    }

    #[test]
    fn classify_disk_available() {
        assert_eq!(classify_disk(false, false, false), DiskRole::Available);
    }

    #[test]
    fn parse_devinfo_single_device() {
        let output = "Label: none  uuid: 375d490e-a0f9-4d5a-a187-f815847ba09a\n\
                       \tTotal devices 1 FS bytes used 694080000000\n\
                       \tdevid    1 size 797998841856 used 700000000000 path /dev/mapper/root\n";
        let devices = parse_devinfo(output);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].devid, 1);
        assert_eq!(devices[0].path, "/dev/mapper/root");
        assert_eq!(devices[0].size_bytes, 797998841856);
        assert_eq!(devices[0].used_bytes, 700000000000);
        assert!(!devices[0].missing);
    }

    #[test]
    fn parse_devinfo_multiple_devices_and_missing() {
        let output = "Label: none  uuid: abc\n\
                       \tTotal devices 2 FS bytes used 100\n\
                       \tdevid    1 size 8000000000 used 4000000000 path /dev/vda7\n\
                       \tdevid    2 size 4000000000 used 1000000000 path /dev/vdb MISSING\n";
        let devices = parse_devinfo(output);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].path, "/dev/vda7");
        assert!(!devices[0].missing);
        assert_eq!(devices[1].path, "/dev/vdb");
        assert!(devices[1].missing);
    }

    #[test]
    fn parse_devinfo_ignores_non_devid_lines() {
        let output = "Label: none  uuid: abc\n\tTotal devices 1 FS bytes used 100\n";
        assert!(parse_devinfo(output).is_empty());
    }

    #[test]
    fn parse_allocation_sums_across_block_group_types() {
        let used = parse_allocation(&["1000", "200", "50"]);
        assert_eq!(used, 1250);
    }

    #[test]
    fn parse_allocation_tolerates_unreadable_files() {
        let used = parse_allocation(&["1000", "", "not-a-number"]);
        assert_eq!(used, 1000);
    }

    fn device(path: &str, size_bytes: u64) -> PoolDevice {
        PoolDevice { devid: 1, path: path.to_owned(), size_bytes, used_bytes: 0, missing: false }
    }

    #[test]
    fn remove_keeps_enough_capacity_when_remaining_devices_fit_usage() {
        let devices = vec![device("/dev/vda7", 8_000_000_000), device("/dev/vdb", 4_000_000_000)];
        assert!(remove_keeps_enough_capacity(&devices, "/dev/vdb", 8_000_000_000));
    }

    #[test]
    fn remove_keeps_enough_capacity_rejects_when_remaining_too_small() {
        let devices = vec![device("/dev/vda7", 8_000_000_000), device("/dev/vdb", 4_000_000_000)];
        assert!(!remove_keeps_enough_capacity(&devices, "/dev/vdb", 9_000_000_000));
    }
}
