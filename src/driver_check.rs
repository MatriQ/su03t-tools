//! Read-only macOS USB/serial binding diagnostics. No serial handles are opened.
//! A missing device is not evidence of a missing driver. Existing callout nodes
//! confirm a binding, regardless of whether macOS or a vendor supplies the driver.
use anyhow::{Context, Result, ensure};
use plist::{Dictionary, Value};
use std::{collections::BTreeSet, io::Cursor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Wch,
    SiliconLabs,
    Ftdi,
}
impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Self::Wch => "WCH CH34x / CH91xx",
            Self::SiliconLabs => "Silicon Labs CP210x",
            Self::Ftdi => "FTDI USB Serial",
        }
    }
    pub fn guide_url(self) -> &'static str {
        match self {
            Self::Wch => "https://github.com/WCHSoftGroup/ch34xser_macos",
            Self::SiliconLabs => {
                "https://www.silabs.com/software-and-tools/usb-to-uart-bridge-vcp-drivers"
            }
            Self::Ftdi => "https://ftdichip.com/drivers/vcp-drivers/",
        }
    }
    fn identify(vid: u16, pid: u16) -> Option<Self> {
        // Match serial product IDs, not vendors: WCH and FTDI also sell non-UART
        // devices. These identifiers are facts, not a guarantee of chip identity.
        match (vid, pid) {
            (
                0x1a86,
                0x7522 | 0x7523 | 0x5523 | 0x55d4 | 0x55d3 | 0x55d2 | 0x55d5 | 0x55d6 | 0x55d7
                | 0x55d8,
            )
            | (0x4348, 0x5523) => Some(Self::Wch),
            (0x10c4, 0xea60 | 0xea70 | 0xea71) => Some(Self::SiliconLabs),
            (0x0403, 0x6001 | 0x6010 | 0x6011 | 0x6014 | 0x6015) => Some(Self::Ftdi),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub family: Option<Family>,
    pub ports: Vec<String>,
    pub drivers: Vec<String>,
}
impl Adapter {
    pub fn ready(&self) -> bool {
        !self.ports.is_empty()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ready,
    NeedsAttention,
    NotDetected,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub adapters: Vec<Adapter>,
}
impl Report {
    pub fn status(&self) -> Status {
        if self.adapters.is_empty() {
            Status::NotDetected
        } else if self.adapters.iter().any(|a| !a.ready()) {
            Status::NeedsAttention
        } else {
            Status::Ready
        }
    }
}
fn string<'a>(node: &'a Dictionary, key: &str) -> Option<&'a str> {
    node.get(key)?.as_string()
}
fn number(node: &Dictionary, key: &str) -> Option<u64> {
    node.get(key)?.as_unsigned_integer()
}
fn usb_device(node: &Dictionary) -> bool {
    matches!(
        string(node, "IOObjectClass"),
        Some("IOUSBHostDevice" | "IOUSBDevice")
    )
}
fn children(node: &Dictionary) -> &[Value] {
    node.get("IORegistryEntryChildren")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}
fn serial_children(
    node: &Dictionary,
    parent_driver: Option<&str>,
    ports: &mut BTreeSet<String>,
    drivers: &mut BTreeSet<String>,
) {
    let provider = string(node, "IOUserServerName")
        .or_else(|| {
            string(node, "CFBundleIdentifier").filter(|id| {
                !id.starts_with("com.apple.iokit.IOUSB") && *id != "com.apple.iokit.IOSerialFamily"
            })
        })
        .or(parent_driver);
    if let Some(path) = string(node, "IOCalloutDevice").filter(|p| p.starts_with("/dev/cu.")) {
        ports.insert(path.into());
        if let Some(provider) = provider {
            drivers.insert(provider.into());
        }
    }
    for child in children(node).iter().filter_map(Value::as_dictionary) {
        // Never credit a child USB device's port to a hub or another adapter.
        if !usb_device(child) {
            serial_children(child, provider, ports, drivers);
        }
    }
}
fn visit(node: &Value, seen: &mut BTreeSet<u64>, adapters: &mut Vec<Adapter>) {
    if let Some(nodes) = node.as_array() {
        for node in nodes {
            visit(node, seen, adapters);
        }
    } else if let Some(node) = node.as_dictionary() {
        if usb_device(node)
            && let (Some(vid), Some(pid)) = (number(node, "idVendor"), number(node, "idProduct"))
            && let (Ok(vid), Ok(pid)) = (u16::try_from(vid), u16::try_from(pid))
        {
            let family = Family::identify(vid, pid);
            let mut ports = BTreeSet::new();
            let mut drivers = BTreeSet::new();
            serial_children(node, None, &mut ports, &mut drivers);
            let duplicate = number(node, "IORegistryEntryID").is_some_and(|id| !seen.insert(id));
            if !duplicate && (family.is_some() || !ports.is_empty()) {
                let name = string(node, "USB Product Name")
                    .or_else(|| string(node, "IORegistryEntryName"))
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| family.map(Family::name).unwrap_or("USB Serial"));
                adapters.push(Adapter {
                    name: name.into(),
                    vid,
                    pid,
                    family,
                    ports: ports.into_iter().collect(),
                    drivers: drivers.into_iter().collect(),
                });
            }
        }
        for child in children(node) {
            visit(child, seen, adapters);
        }
    }
}
pub fn parse_registry(bytes: &[u8]) -> Result<Report> {
    let root = Value::from_reader(Cursor::new(bytes)).context("parse macOS USB registry")?;
    ensure!(
        root.as_array().is_some()
            || root.as_dictionary().is_some_and(|d| {
                d.contains_key("IORegistryEntryChildren") || d.contains_key("IOObjectClass")
            }),
        "unexpected macOS USB registry format"
    );
    let mut report = Report::default();
    visit(&root, &mut BTreeSet::new(), &mut report.adapters);
    Ok(report)
}

#[cfg(target_os = "macos")]
pub fn scan() -> Result<Report> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };
    const LIMIT: usize = 32 * 1024 * 1024;
    let mut child = Command::new("/usr/sbin/ioreg")
        .args(["-a", "-r", "-c", "IOUSBHostDevice", "-l"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("start macOS USB registry check")?;
    let mut stdout = child.stdout.take().context("read USB registry output")?;
    let reader = thread::spawn(move || {
        let mut data = Vec::new();
        stdout
            .by_ref()
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut data)?;
        Ok::<_, std::io::Error>(data)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => break Err(anyhow::anyhow!("macOS USB registry check timed out")),
            Err(e) => break Err(e.into()),
        }
    };
    if status.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("USB registry reader stopped"))??;
    ensure!(
        bytes.len() <= LIMIT,
        "macOS USB registry output exceeds 32 MiB"
    );
    ensure!(status?.success(), "macOS USB registry check failed");
    let mut report = parse_registry(&bytes)?;
    // An IORegistry entry can briefly outlive its device node during unplugging.
    // Metadata is sufficient; opening a port here could reset a connected board.
    for adapter in &mut report.adapters {
        adapter
            .ports
            .retain(|path| std::path::Path::new(path).exists());
        if adapter.ports.is_empty() {
            adapter.drivers.clear();
        }
    }
    Ok(report)
}
#[cfg(not(target_os = "macos"))]
pub fn scan() -> Result<Report> {
    anyhow::bail!("automatic driver checks are available on macOS")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(class: &str, properties: &[(&str, Value)], child_nodes: Vec<Value>) -> Value {
        let mut data = Dictionary::new();
        data.insert("IOObjectClass".into(), class.into());
        for (key, value) in properties {
            data.insert((*key).into(), value.clone());
        }
        if !child_nodes.is_empty() {
            data.insert("IORegistryEntryChildren".into(), Value::Array(child_nodes));
        }
        Value::Dictionary(data)
    }
    fn usb(id: u64, vid: u16, pid: u16, child_nodes: Vec<Value>) -> Value {
        node(
            "IOUSBHostDevice",
            &[
                ("IORegistryEntryID", id.into()),
                ("idVendor", vid.into()),
                ("idProduct", pid.into()),
                ("USB Product Name", "test adapter".into()),
            ],
            child_nodes,
        )
    }
    fn serial(path: &str, driver: &str) -> Value {
        node(
            "IOUserSerial",
            &[("IOUserServerName", driver.into())],
            vec![node(
                "IOSerialBSDClient",
                &[
                    ("IOCalloutDevice", path.into()),
                    (
                        "CFBundleIdentifier",
                        "com.apple.iokit.IOSerialFamily".into(),
                    ),
                ],
                vec![],
            )],
        )
    }
    fn report(roots: Vec<Value>) -> Report {
        let mut bytes = Vec::new();
        Value::Array(roots).to_writer_xml(&mut bytes).unwrap();
        parse_registry(&bytes).unwrap()
    }
    #[test]
    fn system_driver_is_ready_without_a_vendor_extension() {
        let report = report(vec![usb(
            1,
            0x1a86,
            0x7523,
            vec![serial(
                "/dev/cu.usbserial-test",
                "com.apple.DriverKit-AppleUSBCHCOM",
            )],
        )]);
        assert_eq!(report.status(), Status::Ready);
        assert_eq!(
            report.adapters[0].drivers,
            ["com.apple.DriverKit-AppleUSBCHCOM"]
        );
        assert_eq!(report.adapters[0].family, Some(Family::Wch));
    }
    #[test]
    fn recognized_usb_without_a_serial_interface_needs_attention() {
        let report = report(vec![usb(1, 0x1a86, 0x7523, vec![])]);
        assert_eq!(report.status(), Status::NeedsAttention);
        assert!(report.adapters[0].drivers.is_empty());
        assert_eq!(report.adapters[0].family, Some(Family::Wch));
    }
    #[test]
    fn absent_adapter_is_not_a_missing_driver() {
        assert_eq!(report(vec![]).status(), Status::NotDetected);
        let report = report(vec![node(
            "IOSerialBSDClient",
            &[("IOCalloutDevice", "/dev/cu.Bluetooth-Incoming-Port".into())],
            vec![],
        )]);
        assert_eq!(report.status(), Status::NotDetected);
    }
    #[test]
    fn non_serial_products_from_the_same_vendor_do_not_trigger_a_warning() {
        let report = report(vec![
            usb(1, 0x1a86, 0xe020, vec![]),
            usb(2, 0x0403, 0x1234, vec![]),
        ]);
        assert_eq!(report.status(), Status::NotDetected);
    }
    #[test]
    fn one_working_adapter_does_not_hide_an_unbound_identical_adapter() {
        let report = report(vec![
            usb(
                1,
                0x1a86,
                0x7523,
                vec![serial("/dev/cu.first", "cn.wch.CH34xVCPDriver")],
            ),
            usb(2, 0x1a86, 0x7523, vec![]),
        ]);
        assert_eq!(report.status(), Status::NeedsAttention);
        assert!(report.adapters[0].ready());
        assert!(!report.adapters[1].ready());
    }
    #[test]
    fn a_descendant_usb_device_cannot_supply_its_parents_serial_port() {
        let child = usb(
            2,
            0x10c4,
            0xea60,
            vec![serial("/dev/cu.child", "com.silabs.driver")],
        );
        let report = report(vec![usb(1, 0x1a86, 0x7523, vec![child.clone()]), child]);
        assert_eq!(report.adapters.len(), 2);
        assert!(!report.adapters[0].ready());
        assert_eq!(report.adapters[1].ports, ["/dev/cu.child"]);
    }
    #[test]
    fn repeated_registry_roots_are_deduplicated_by_registry_id() {
        let adapter = usb(
            123,
            0x1a86,
            0x7523,
            vec![serial("/dev/cu.test", "cn.wch.driver")],
        );
        let report = report(vec![adapter.clone(), adapter]);
        assert_eq!(report.adapters.len(), 1);
        assert_eq!(report.status(), Status::Ready);
    }
    #[test]
    fn custom_usb_ids_with_a_serial_interface_are_ready_without_guessing_a_vendor() {
        let report = report(vec![usb(
            1,
            0xabcd,
            0x1234,
            vec![serial("/dev/cu.custom", "custom.driver")],
        )]);
        assert_eq!(report.status(), Status::Ready);
        assert_eq!(report.adapters[0].family, None);
    }
    #[test]
    fn registry_parse_failures_do_not_become_absent_device_reports() {
        assert!(parse_registry(b"not a plist").is_err());
        let mut bytes = Vec::new();
        Value::String("unexpected root".into())
            .to_writer_xml(&mut bytes)
            .unwrap();
        assert!(parse_registry(&bytes).is_err());
    }
    #[test]
    fn dictionary_roots_and_multiport_adapters_are_supported() {
        let root = usb(
            1,
            0x0403,
            0x6010,
            vec![
                serial("/dev/cu.uart-A", "com.apple.driver.AppleUSBFTDI"),
                serial("/dev/cu.uart-B", "com.apple.driver.AppleUSBFTDI"),
            ],
        );
        let mut bytes = Vec::new();
        root.to_writer_binary(&mut bytes).unwrap();
        let report = parse_registry(&bytes).unwrap();
        assert_eq!(report.status(), Status::Ready);
        assert_eq!(report.adapters[0].ports.len(), 2);
        assert_eq!(
            report.adapters[0].drivers,
            ["com.apple.driver.AppleUSBFTDI"]
        );
    }
}
