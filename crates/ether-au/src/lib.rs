//! Audio Unit hosting (native; real only on macOS). Owned by the `au` node; see
//! `docs/PLUGIN-FORMATS.md`.
//!
//! [`AuFormat`] is the [`PluginFormatHost`] for AUs. AUs are found through the system
//! `AudioComponent` registry (`AudioComponentFindNext`), not by walking folders: that covers
//! Apple's built-in units (AUDelay, AULowpass, DLSMusicDevice, ...), `.component` bundles and
//! AUv3 app extensions alike. The id convention ([`AuComponentId`]) is real; registry
//! listing, scanning and instantiation are stubs (`PluginError::Unsupported`, empty listing)
//! until the `au` node implements them. Off macOS everything is `Unsupported` for good.
//!
//! # Ids
//! `type:subtype:manufacturer`, each an `AudioComponentDescription` four-char code
//! (`aufx:dely:appl` = AUDelay). Printable ASCII bytes are kept verbatim (case-sensitive,
//! spaces kept); any other byte is written `\xHH`, so ids are ASCII and lossless.
//! `PluginDescriptor.path` for an AU is the scan target, i.e. the same id string.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use ether_core::plugin::{PluginController, PluginError};
use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::PluginDescriptor;
use ether_plugin_host::{PluginFormatHost, unsupported};

/// An `AudioComponentDescription`'s identifying codes (`componentType`,
/// `componentSubType`, `componentManufacturer`), as big-endian four-char bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AuComponentId {
    pub component_type: [u8; 4],
    pub subtype: [u8; 4],
    pub manufacturer: [u8; 4],
}

fn write_code(out: &mut String, code: &[u8; 4]) {
    for &b in code {
        if (0x20..0x7F).contains(&b) && b != b'\\' && b != b':' {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\x{b:02X}"));
        }
    }
}

fn parse_code(s: &str) -> Option<[u8; 4]> {
    let mut out = Vec::with_capacity(4);
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            let hex = s.get(i + 2..i + 4).filter(|_| b.get(i + 1) == Some(&b'x'))?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 4;
        } else if (0x20..0x7F).contains(&b[i]) {
            out.push(b[i]);
            i += 1;
        } else {
            return None;
        }
    }
    out.try_into().ok()
}

impl AuComponentId {
    /// From the `OSType` values of an `AudioComponentDescription`.
    pub fn from_os_types(component_type: u32, subtype: u32, manufacturer: u32) -> Self {
        Self {
            component_type: component_type.to_be_bytes(),
            subtype: subtype.to_be_bytes(),
            manufacturer: manufacturer.to_be_bytes(),
        }
    }

    /// `(componentType, componentSubType, componentManufacturer)` as `OSType`s.
    pub fn os_types(&self) -> (u32, u32, u32) {
        (
            u32::from_be_bytes(self.component_type),
            u32::from_be_bytes(self.subtype),
            u32::from_be_bytes(self.manufacturer),
        )
    }

    /// Parse a plugin id (`type:subtype:manufacturer`). `None` if malformed.
    pub fn parse(id: &str) -> Option<Self> {
        // `:` is escaped inside codes, so a plain split is exact.
        let mut parts = id.split(':');
        let (t, s, m) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            component_type: parse_code(t)?,
            subtype: parse_code(s)?,
            manufacturer: parse_code(m)?,
        })
    }

    /// Device category implied by the component type: `aumu` (music device) = instrument,
    /// `aumi` (MIDI processor) = note effect, everything else (`aufx`, `aumf`, ...) = audio
    /// effect.
    pub fn category(&self) -> ether_core::protocol::devices::DeviceCategory {
        use ether_core::protocol::devices::DeviceCategory;
        match &self.component_type {
            b"aumu" => DeviceCategory::Instrument,
            b"aumi" => DeviceCategory::NoteEffect,
            _ => DeviceCategory::AudioEffect,
        }
    }
}

impl std::fmt::Display for AuComponentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::with_capacity(14);
        write_code(&mut s, &self.component_type);
        s.push(':');
        write_code(&mut s, &self.subtype);
        s.push(':');
        write_code(&mut s, &self.manufacturer);
        f.write_str(&s)
    }
}

/// Conventional AU install folders (`.component` bundles), user first. For information and
/// rescan triggers only: discovery goes through the component registry. Empty off macOS.
pub fn default_search_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut paths = Vec::new();
        if let Some(home) = ether_plugin_host::bundles::home_dir() {
            paths.push(home.join("Library/Audio/Plug-Ins/Components"));
        }
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins/Components"));
        ether_plugin_host::bundles::dedup_paths(paths)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

fn not_here(what: &str) -> PluginError {
    if cfg!(target_os = "macos") {
        unsupported(PluginFormat::Au, what)
    } else {
        PluginError::Unsupported("Audio Units are only available on macOS".into())
    }
}

/// Audio Units as a [`PluginFormatHost`].
#[derive(Clone, Copy, Debug, Default)]
pub struct AuFormat;

impl PluginFormatHost for AuFormat {
    fn format(&self) -> PluginFormat {
        PluginFormat::Au
    }
    fn default_search_paths(&self) -> Vec<PathBuf> {
        default_search_paths()
    }
    /// AUs are not discovered by path (see [`AuFormat::discover_registry`]).
    fn discover(&self, paths: &[PathBuf]) -> Vec<PathBuf> {
        let _ = paths;
        Vec::new()
    }
    /// One target per registered component (its id). Stub: empty until the `au` node lists
    /// the registry.
    fn discover_registry(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn claims(&self, target: &Path) -> bool {
        target.to_str().is_some_and(|s| AuComponentId::parse(s).is_some())
    }
    fn scan(&self, target: &Path) -> Result<Vec<PluginDescriptor>, PluginError> {
        let _ = target;
        Err(not_here("scanning"))
    }
    fn instantiate(
        &self,
        path: &Path,
        plugin_id: &str,
    ) -> Result<Box<dyn PluginController>, PluginError> {
        let _ = (path, plugin_id);
        Err(not_here("loading"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::devices::DeviceCategory;

    #[test]
    fn ids_round_trip() {
        let id = AuComponentId::parse("aufx:dely:appl").unwrap();
        assert_eq!(&id.component_type, b"aufx");
        assert_eq!(id.to_string(), "aufx:dely:appl");
        assert_eq!(
            id.os_types(),
            (
                u32::from_be_bytes(*b"aufx"),
                u32::from_be_bytes(*b"dely"),
                u32::from_be_bytes(*b"appl")
            )
        );
        let (t, s, m) = id.os_types();
        assert_eq!(AuComponentId::from_os_types(t, s, m), id);
        assert_eq!(id.category(), DeviceCategory::AudioEffect);
        assert_eq!(
            AuComponentId::parse("aumu:dls :appl").unwrap().category(),
            DeviceCategory::Instrument
        );

        // Escapes: non-printable/non-ASCII bytes, `:` and `\` inside codes.
        let odd = AuComponentId {
            component_type: *b"aumi",
            subtype: [b'a', b':', 0xA9, b'\\'],
            manufacturer: *b"Ab C",
        };
        let s = odd.to_string();
        assert_eq!(s, "aumi:a\\x3A\\xA9\\x5C:Ab C");
        assert!(s.is_ascii());
        assert_eq!(AuComponentId::parse(&s), Some(odd));
        assert_eq!(odd.category(), DeviceCategory::NoteEffect);

        for bad in ["aufx:dely", "aufx:dely:appl:x", "aufx:del:appl", "aufx:delay:appl", "aufx:d\\x4:appl", "aufx:dél:appl", ""] {
            assert_eq!(AuComponentId::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn stub_reports_unsupported() {
        let f = AuFormat;
        assert_eq!(f.format(), PluginFormat::Au);
        assert!(f.claims(Path::new("aufx:dely:appl")));
        assert!(!f.claims(Path::new("/p/A.vst3")));
        assert!(f.discover(&default_search_paths()).is_empty());
        assert!(f.discover_registry().is_empty());
        assert_eq!(default_search_paths().is_empty(), !cfg!(target_os = "macos"));
        let e = f.scan(Path::new("aufx:dely:appl")).unwrap_err();
        assert!(matches!(e, PluginError::Unsupported(_)), "{e:?}");
        let e = f
            .instantiate(Path::new("aufx:dely:appl"), "aufx:dely:appl")
            .err()
            .unwrap();
        assert!(matches!(e, PluginError::Unsupported(_)), "{e:?}");
    }
}
