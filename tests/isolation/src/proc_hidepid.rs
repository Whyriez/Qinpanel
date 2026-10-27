/// Represents the hidepid configuration level on `/proc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HidepidLevel {
    /// hidepid=0: Default Linux behavior, all processes visible to everyone.
    AllVisible = 0,
    /// hidepid=1: Basic info visible, but cmdline/status restricted.
    RestrictedInfo = 1,
    /// hidepid=2 / hidepid=invisible: Processes owned by other users are invisible (ENOENT/EACCES).
    Invisible = 2,
}

/// Parses the `/proc` mount entry line from `/proc/mounts` or `/etc/fstab`.
pub fn parse_proc_mount_options(mount_line: &str) -> Option<HidepidLevel> {
    // Format: "proc /proc proc rw,nosuid,nodev,noexec,relatime,hidepid=2 0 0"
    let parts: Vec<&str> = mount_line.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }

    if parts[1] != "/proc" || parts[2] != "proc" {
        return None;
    }

    let options = parts[3];
    for opt in options.split(',') {
        if opt == "hidepid=2" || opt == "hidepid=invisible" {
            return Some(HidepidLevel::Invisible);
        } else if opt == "hidepid=1" {
            return Some(HidepidLevel::RestrictedInfo);
        } else if opt == "hidepid=0" || opt == "hidepid=off" {
            return Some(HidepidLevel::AllVisible);
        }
    }

    // Default if hidepid is omitted is 0
    Some(HidepidLevel::AllVisible)
}

/// Determines whether an accessor user can inspect a process in `/proc/<pid>`.
pub fn can_inspect_process(
    accessor_uid: u32,
    process_owner_uid: u32,
    hidepid_level: HidepidLevel,
) -> bool {
    // Root (UID 0) can always inspect any process regardless of hidepid
    if accessor_uid == 0 {
        return true;
    }

    // A user can always inspect their own processes
    if accessor_uid == process_owner_uid {
        return true;
    }

    // Under hidepid=2 (Invisible), other users' processes cannot be seen
    match hidepid_level {
        HidepidLevel::Invisible => false,
        HidepidLevel::RestrictedInfo => true, // Visible in list, but files inside restricted
        HidepidLevel::AllVisible => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_proc_mount_options() {
        let line_hardened = "proc /proc proc rw,nosuid,nodev,noexec,relatime,hidepid=2 0 0";
        assert_eq!(
            parse_proc_mount_options(line_hardened),
            Some(HidepidLevel::Invisible)
        );

        let line_invisible_alias =
            "proc /proc proc rw,nosuid,nodev,noexec,relatime,hidepid=invisible 0 0";
        assert_eq!(
            parse_proc_mount_options(line_invisible_alias),
            Some(HidepidLevel::Invisible)
        );

        let line_default = "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0";
        assert_eq!(
            parse_proc_mount_options(line_default),
            Some(HidepidLevel::AllVisible)
        );
    }

    #[test]
    fn test_process_inspection_under_hidepid_2() {
        let alice_uid = 10001;
        let bob_uid = 10002;
        let root_uid = 0;

        // Under hidepid=2:
        // Alice inspecting Alice's process -> allowed
        assert!(can_inspect_process(
            alice_uid,
            alice_uid,
            HidepidLevel::Invisible
        ));

        // Alice inspecting Bob's process -> DENIED (invisible)
        assert!(!can_inspect_process(
            alice_uid,
            bob_uid,
            HidepidLevel::Invisible
        ));

        // Bob inspecting Alice's process -> DENIED (invisible)
        assert!(!can_inspect_process(
            bob_uid,
            alice_uid,
            HidepidLevel::Invisible
        ));

        // Root inspecting Alice or Bob's process -> allowed
        assert!(can_inspect_process(
            root_uid,
            alice_uid,
            HidepidLevel::Invisible
        ));
        assert!(can_inspect_process(
            root_uid,
            bob_uid,
            HidepidLevel::Invisible
        ));
    }

    #[test]
    fn test_unhardened_leak_without_hidepid() {
        let alice_uid = 10001;
        let bob_uid = 10002;

        // Without hidepid (AllVisible):
        // Alice could see Bob's process in /proc
        assert!(can_inspect_process(
            alice_uid,
            bob_uid,
            HidepidLevel::AllVisible
        ));
    }
}
